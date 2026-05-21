use crate::clipboard::{ClipboardContent, ClipboardReader};
use crate::config::AppConfig;
use crate::hotkey::{Hotkey, WindowsPhysicalKeyState};
use crate::remote_path::generate_remote_image_path;
use crate::ssh_args::SshInvocation;
use crate::template::render_template;
use crate::uploader::{CommandRunner, Uploader};
use anyhow::{Context, Result};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::thread;
use std::time::Duration;
use tempfile::NamedTempFile;

const HOTKEY_POLL_TIMEOUT: Duration = Duration::from_millis(50);
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(250);

pub fn upload_clipboard_image_path<C, R>(
    config: &AppConfig,
    invocation: &SshInvocation,
    clipboard: &mut C,
    uploader: &mut Uploader<R>,
) -> Result<Option<String>>
where
    C: ClipboardReader,
    R: CommandRunner,
{
    let mut cache = ClipboardImageUploadCache::default();
    upload_clipboard_image_path_cached(config, invocation, clipboard, uploader, &mut cache)
}

pub fn upload_clipboard_image_path_cached<C, R>(
    config: &AppConfig,
    invocation: &SshInvocation,
    clipboard: &mut C,
    uploader: &mut Uploader<R>,
    cache: &mut ClipboardImageUploadCache,
) -> Result<Option<String>>
where
    C: ClipboardReader,
    R: CommandRunner,
{
    let ClipboardContent::Image(image) = clipboard.read()? else {
        return Ok(None);
    };

    let fingerprint = ClipboardImageFingerprint::from_image(&image);
    if let Some(cached) = cache.last.as_ref() {
        if cached.fingerprint == fingerprint {
            return Ok(Some(cached.rendered_path.clone()));
        }
    }

    let temp = NamedTempFile::new()?;
    image.save_png(temp.path())?;
    let remote = generate_remote_image_path(
        &config.remote_dir,
        &config.filename_pattern,
        &config.image_format,
    );
    uploader.upload(invocation, temp.path(), &remote)?;
    let rendered_path = render_template(&config.template, &remote);
    cache.last = Some(CachedClipboardImageUpload {
        fingerprint,
        rendered_path: rendered_path.clone(),
    });
    Ok(Some(rendered_path))
}

#[derive(Default)]
pub struct ClipboardImageUploadCache {
    last: Option<CachedClipboardImageUpload>,
}

struct CachedClipboardImageUpload {
    fingerprint: ClipboardImageFingerprint,
    rendered_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ClipboardImageFingerprint {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

impl ClipboardImageFingerprint {
    fn from_image(image: &crate::clipboard::ClipboardImage) -> Self {
        Self {
            width: image.width,
            height: image.height,
            rgba: image.rgba.clone(),
        }
    }
}

pub fn run_terminal_session(config: AppConfig, invocation: SshInvocation) -> Result<()> {
    let hotkey = Hotkey::parse(&config.paste_hotkey)?;
    let running = Arc::new(AtomicBool::new(true));
    let worker_running = Arc::clone(&running);
    let worker_config = config.clone();
    let worker_invocation = invocation.clone();

    let hotkey_thread = thread::spawn(move || {
        if let Err(error) =
            run_image_hotkey_worker(worker_running, worker_config, worker_invocation, hotkey)
        {
            eprintln!("\r\nipssh hotkey worker failed: {error:#}\r");
        }
    });

    let status = Command::new("ssh.exe")
        .args(&invocation.ssh_args)
        .status()
        .with_context(|| "failed to run ssh.exe")?;

    running.store(false, Ordering::SeqCst);
    let _ = hotkey_thread.join();

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

fn run_image_hotkey_worker(
    running: Arc<AtomicBool>,
    config: AppConfig,
    invocation: SshInvocation,
    hotkey: Hotkey,
) -> Result<()> {
    use crate::clipboard::SystemClipboard;
    use crate::uploader::SystemCommandRunner;

    let physical_keys = WindowsPhysicalKeyState;
    let hotkey_events = KeyboardHotkeyEvents::new(&hotkey)?;
    let mut clipboard = SystemClipboard::new()?;
    let mut uploader = Uploader::new(SystemCommandRunner);
    let mut inserter = ClipboardPastePathInserter;
    let mut upload_cache = ClipboardImageUploadCache::default();

    while running.load(Ordering::SeqCst) {
        if hotkey_events.recv_timeout(HOTKEY_POLL_TIMEOUT) {
            match upload_clipboard_image_path_cached(
                &config,
                &invocation,
                &mut clipboard,
                &mut uploader,
                &mut upload_cache,
            ) {
                Ok(Some(path)) => {
                    wait_for_hotkey_release(&hotkey_events.spec, &physical_keys, &running);
                    hotkey_events.drain();
                    inserter.insert(&path)?;
                }
                Ok(None) => {}
                Err(error) => eprintln!("\r\nipssh paste failed: {error:#}\r"),
            }
        }
        thread::sleep(HOTKEY_POLL_TIMEOUT);
    }

    Ok(())
}

fn wait_for_hotkey_release(
    spec: &KeyboardHotkeySpec,
    physical_keys: &WindowsPhysicalKeyState,
    running: &AtomicBool,
) {
    while running.load(Ordering::SeqCst) && spec.is_down(physical_keys) {
        thread::sleep(HOTKEY_POLL_TIMEOUT);
    }
}

#[derive(Debug, Clone, Copy)]
struct KeyboardHotkeySpec {
    key_vk: i32,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

impl KeyboardHotkeySpec {
    fn from_hotkey(hotkey: &Hotkey) -> Self {
        use crossterm::event::KeyModifiers;

        Self {
            key_vk: hotkey.key.to_ascii_uppercase() as i32,
            ctrl: hotkey.modifiers.contains(KeyModifiers::CONTROL),
            shift: hotkey.modifiers.contains(KeyModifiers::SHIFT),
            alt: hotkey.modifiers.contains(KeyModifiers::ALT),
        }
    }

    fn matches<S>(&self, key_vk: i32, is_pressed: S) -> bool
    where
        S: Fn(i32) -> bool,
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_MENU, VK_SHIFT};

        key_vk == self.key_vk
            && (!self.ctrl || is_pressed(VK_CONTROL as i32))
            && (!self.shift || is_pressed(VK_SHIFT as i32))
            && (!self.alt || is_pressed(VK_MENU as i32))
    }

    fn is_down(&self, _physical_keys: &WindowsPhysicalKeyState) -> bool {
        self.matches(self.key_vk, is_key_pressed)
    }
}

#[derive(Debug, Clone, Copy)]
struct ForegroundWindowGate {
    allowed_window: Option<usize>,
}

impl ForegroundWindowGate {
    fn current() -> Self {
        Self {
            allowed_window: current_foreground_window_id(),
        }
    }

    fn allows(&self, current_window: Option<usize>) -> bool {
        match self.allowed_window {
            Some(allowed_window) => current_window == Some(allowed_window),
            None => true,
        }
    }
}

struct KeyboardHotkeyEvents {
    spec: KeyboardHotkeySpec,
    receiver: Receiver<()>,
}

impl KeyboardHotkeyEvents {
    fn new(hotkey: &Hotkey) -> Result<Self> {
        let spec = KeyboardHotkeySpec::from_hotkey(hotkey);
        let receiver = install_keyboard_hook(spec)?;
        Ok(Self { spec, receiver })
    }

    fn recv_timeout(&self, timeout: Duration) -> bool {
        self.receiver.recv_timeout(timeout).is_ok()
    }

    fn drain(&self) {
        while self.receiver.try_recv().is_ok() {}
    }
}

#[cfg(windows)]
#[derive(Clone)]
struct KeyboardHookState {
    spec: KeyboardHotkeySpec,
    foreground_gate: ForegroundWindowGate,
    sender: mpsc::Sender<()>,
}

#[cfg(windows)]
static KEYBOARD_HOOK_STATE: Mutex<Option<KeyboardHookState>> = Mutex::new(None);

#[cfg(windows)]
fn install_keyboard_hook(spec: KeyboardHotkeySpec) -> Result<Receiver<()>> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, SetWindowsHookExW, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL,
        WM_KEYDOWN, WM_SYSKEYDOWN,
    };

    let (sender, receiver) = mpsc::channel();
    let (ready_sender, ready_receiver) = mpsc::channel();
    let foreground_gate = ForegroundWindowGate::current();
    thread::spawn(move || {
        if let Ok(mut state) = KEYBOARD_HOOK_STATE.lock() {
            *state = Some(KeyboardHookState {
                spec,
                foreground_gate,
                sender,
            });
        }

        unsafe extern "system" fn hook_proc(
            code: i32,
            wparam: windows_sys::Win32::Foundation::WPARAM,
            lparam: windows_sys::Win32::Foundation::LPARAM,
        ) -> windows_sys::Win32::Foundation::LRESULT {
            if code >= 0 && (wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN) {
                let event = unsafe { *(lparam as *const KBDLLHOOKSTRUCT) };
                if let Ok(state) = KEYBOARD_HOOK_STATE.lock() {
                    if let Some(state) = state.as_ref() {
                        let key_vk = event.vkCode as i32;
                        if state.foreground_gate.allows(current_foreground_window_id())
                            && state.spec.matches(key_vk, is_key_pressed)
                        {
                            let _ = state.sender.send(());
                        }
                    }
                }
            }
            unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
        }

        let hook =
            unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), std::ptr::null_mut(), 0) };
        if hook.is_null() {
            let _ = ready_sender.send(Err(anyhow::anyhow!(
                "failed to install low-level keyboard hook"
            )));
            return;
        }
        let _ = ready_sender.send(Ok(()));

        let mut message = MSG::default();
        while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {}
    });

    ready_receiver
        .recv()
        .context("keyboard hook thread did not start")??;
    Ok(receiver)
}

#[cfg(not(windows))]
fn install_keyboard_hook(_spec: KeyboardHotkeySpec) -> Result<Receiver<()>> {
    let (_sender, receiver) = mpsc::channel();
    Ok(receiver)
}

fn is_key_pressed(key_vk: i32) -> bool {
    unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(key_vk) < 0 }
}

#[cfg(windows)]
fn current_foreground_window_id() -> Option<usize> {
    let window = unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
    if window.is_null() {
        None
    } else {
        Some(window as usize)
    }
}

#[cfg(not(windows))]
fn current_foreground_window_id() -> Option<usize> {
    None
}

trait RemotePathInserter {
    fn insert(&mut self, text: &str) -> Result<()>;
}

struct ClipboardPastePathInserter;

impl RemotePathInserter for ClipboardPastePathInserter {
    fn insert(&mut self, text: &str) -> Result<()> {
        paste_text_via_clipboard(text)
    }
}

#[cfg(windows)]
fn paste_text_via_clipboard(text: &str) -> Result<()> {
    use arboard::{Clipboard, ImageData};
    use std::borrow::Cow;

    let mut clipboard = Clipboard::new().context("failed to open Windows clipboard")?;
    let previous = if let Ok(image) = clipboard.get_image() {
        Some(ClipboardContent::Image(crate::clipboard::ClipboardImage {
            width: image.width,
            height: image.height,
            rgba: image.bytes.into_owned(),
        }))
    } else if let Ok(text) = clipboard.get_text() {
        Some(ClipboardContent::Text(text))
    } else {
        None
    };

    clipboard
        .set_text(text.to_string())
        .context("failed to set clipboard text for paste")?;
    send_shift_insert()?;
    thread::sleep(CLIPBOARD_RESTORE_DELAY);

    if let Some(previous) = previous {
        match previous {
            ClipboardContent::Image(image) => clipboard
                .set_image(ImageData {
                    width: image.width,
                    height: image.height,
                    bytes: Cow::Owned(image.rgba),
                })
                .context("failed to restore clipboard image")?,
            ClipboardContent::Text(text) => clipboard
                .set_text(text)
                .context("failed to restore clipboard text")?,
            ClipboardContent::Empty => {}
        }
    }

    Ok(())
}

#[cfg(not(windows))]
fn paste_text_via_clipboard(_text: &str) -> Result<()> {
    anyhow::bail!("clipboard paste insertion is only supported on Windows")
}

#[cfg(windows)]
fn send_shift_insert() -> Result<()> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, VK_INSERT, VK_SHIFT,
    };

    fn key_input(vk: u16, flags: u32) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    let inputs = [
        key_input(VK_SHIFT, 0),
        key_input(VK_INSERT, 0),
        key_input(VK_INSERT, KEYEVENTF_KEYUP),
        key_input(VK_SHIFT, KEYEVENTF_KEYUP),
    ];
    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };
    if sent != inputs.len() as u32 {
        anyhow::bail!("SendInput sent {sent} of {} input events", inputs.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::{ClipboardImage, ClipboardReader};
    use crate::uploader::{CommandResult, CommandRunner};

    struct FakeClipboard {
        content: ClipboardContent,
    }

    impl ClipboardReader for FakeClipboard {
        fn read(&mut self) -> Result<ClipboardContent> {
            Ok(self.content.clone())
        }
    }

    #[derive(Default)]
    struct FakeRunner {
        calls: usize,
    }

    impl CommandRunner for FakeRunner {
        fn run(&mut self, _program: &str, _args: &[String]) -> Result<CommandResult> {
            self.calls += 1;
            Ok(CommandResult {
                success: true,
                code: Some(0),
                stderr: String::new(),
            })
        }
    }

    #[test]
    fn text_clipboard_on_image_shortcut_is_not_handled() {
        let config = AppConfig::load(None, Default::default()).unwrap();
        let invocation = SshInvocation::parse(vec!["host".to_string()]).unwrap();
        let mut clipboard = FakeClipboard {
            content: ClipboardContent::Text("line1\r\nline2".to_string()),
        };
        let mut uploader = Uploader::new(FakeRunner::default());

        let path = upload_clipboard_image_path(&config, &invocation, &mut clipboard, &mut uploader)
            .unwrap();

        assert_eq!(path, None);
    }

    #[test]
    fn empty_clipboard_on_image_shortcut_is_not_handled() {
        let config = AppConfig::load(None, Default::default()).unwrap();
        let invocation = SshInvocation::parse(vec!["host".to_string()]).unwrap();
        let mut clipboard = FakeClipboard {
            content: ClipboardContent::Empty,
        };
        let mut uploader = Uploader::new(FakeRunner::default());

        let path = upload_clipboard_image_path(&config, &invocation, &mut clipboard, &mut uploader)
            .unwrap();

        assert_eq!(path, None);
    }

    #[test]
    fn image_branch_returns_rendered_remote_path() {
        let config = AppConfig::load(None, Default::default()).unwrap();
        let invocation = SshInvocation::parse(vec!["host".to_string()]).unwrap();
        let mut clipboard = FakeClipboard {
            content: ClipboardContent::Image(ClipboardImage {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 0, 255],
            }),
        };
        let mut uploader = Uploader::new(FakeRunner::default());

        let path = upload_clipboard_image_path(&config, &invocation, &mut clipboard, &mut uploader)
            .unwrap()
            .unwrap();

        assert!(path.starts_with("/tmp/ipssh-images/"));
        assert!(path.ends_with(".png"));
    }

    #[test]
    fn repeated_same_clipboard_image_reuses_cached_path_without_uploading_again() {
        let config = AppConfig::load(None, Default::default()).unwrap();
        let invocation = SshInvocation::parse(vec!["host".to_string()]).unwrap();
        let image = ClipboardImage {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        };
        let mut clipboard = FakeClipboard {
            content: ClipboardContent::Image(image.clone()),
        };
        let mut uploader = Uploader::new(FakeRunner::default());
        let mut cache = ClipboardImageUploadCache::default();

        let first = upload_clipboard_image_path_cached(
            &config,
            &invocation,
            &mut clipboard,
            &mut uploader,
            &mut cache,
        )
        .unwrap();
        clipboard.content = ClipboardContent::Image(image);
        let second = upload_clipboard_image_path_cached(
            &config,
            &invocation,
            &mut clipboard,
            &mut uploader,
            &mut cache,
        )
        .unwrap();

        assert_eq!(second, first);
        assert_eq!(uploader.runner.calls, 2);
    }

    #[test]
    fn changed_clipboard_image_uploads_again() {
        let config = AppConfig::load(None, Default::default()).unwrap();
        let invocation = SshInvocation::parse(vec!["host".to_string()]).unwrap();
        let mut clipboard = FakeClipboard {
            content: ClipboardContent::Image(ClipboardImage {
                width: 1,
                height: 1,
                rgba: vec![0, 0, 0, 255],
            }),
        };
        let mut uploader = Uploader::new(FakeRunner::default());
        let mut cache = ClipboardImageUploadCache::default();

        upload_clipboard_image_path_cached(
            &config,
            &invocation,
            &mut clipboard,
            &mut uploader,
            &mut cache,
        )
        .unwrap();
        clipboard.content = ClipboardContent::Image(ClipboardImage {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 255],
        });
        upload_clipboard_image_path_cached(
            &config,
            &invocation,
            &mut clipboard,
            &mut uploader,
            &mut cache,
        )
        .unwrap();

        assert_eq!(uploader.runner.calls, 4);
    }

    #[test]
    fn keyboard_hotkey_spec_matches_keydown_without_polling_window() {
        let hotkey = Hotkey::parse("ctrl+v").unwrap();
        let spec = KeyboardHotkeySpec::from_hotkey(&hotkey);

        assert!(spec.matches('V' as i32, |key| {
            key == windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL as i32
        }));
    }

    #[test]
    fn keyboard_hotkey_spec_rejects_missing_modifier() {
        let hotkey = Hotkey::parse("ctrl+v").unwrap();
        let spec = KeyboardHotkeySpec::from_hotkey(&hotkey);

        assert!(!spec.matches('V' as i32, |_key| false));
    }

    #[test]
    fn foreground_window_gate_allows_startup_window() {
        let gate = ForegroundWindowGate {
            allowed_window: Some(10),
        };

        assert!(gate.allows(Some(10)));
    }

    #[test]
    fn foreground_window_gate_rejects_other_windows() {
        let gate = ForegroundWindowGate {
            allowed_window: Some(10),
        };

        assert!(!gate.allows(Some(11)));
        assert!(!gate.allows(None));
    }

    #[test]
    fn foreground_window_gate_without_startup_window_does_not_block_hotkeys() {
        let gate = ForegroundWindowGate {
            allowed_window: None,
        };

        assert!(gate.allows(Some(11)));
        assert!(gate.allows(None));
    }
}
