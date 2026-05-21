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
const CLIPBOARD_RESTORE_RETRY_DELAY: Duration = Duration::from_millis(50);
const CLIPBOARD_RESTORE_ATTEMPTS: usize = 40;

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
    let ClipboardContent::Image(image) = clipboard.read()? else {
        return Ok(None);
    };

    let temp = NamedTempFile::new()?;
    image.save_png(temp.path())?;
    let remote = generate_remote_image_path(
        &config.remote_dir,
        &config.filename_pattern,
        &config.image_format,
    );
    uploader.upload(invocation, temp.path(), &remote)?;
    Ok(Some(render_template(&config.template, &remote)))
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

    while running.load(Ordering::SeqCst) {
        if hotkey_events.recv_timeout(HOTKEY_POLL_TIMEOUT) {
            match upload_clipboard_image_path(&config, &invocation, &mut clipboard, &mut uploader) {
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
    thread::spawn(move || {
        if let Ok(mut state) = KEYBOARD_HOOK_STATE.lock() {
            *state = Some(KeyboardHookState { spec, sender });
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
                        if state.spec.matches(key_vk, is_key_pressed) {
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
    let mut clipboard = SystemPasteClipboard::new()?;
    paste_text_via_clipboard_with(&mut clipboard, text, send_shift_insert, |duration| {
        thread::sleep(duration)
    })
}

#[cfg(not(windows))]
fn paste_text_via_clipboard(_text: &str) -> Result<()> {
    anyhow::bail!("clipboard paste insertion is only supported on Windows")
}

trait PasteClipboard {
    fn read_content(&mut self) -> Result<Option<ClipboardContent>>;
    fn set_text(&mut self, text: String) -> Result<()>;
    fn set_content(&mut self, content: Option<ClipboardContent>) -> Result<()>;
}

fn paste_text_via_clipboard_with<C, P, S>(
    clipboard: &mut C,
    text: &str,
    mut send_paste: P,
    mut sleep: S,
) -> Result<()>
where
    C: PasteClipboard,
    P: FnMut() -> Result<()>,
    S: FnMut(Duration),
{
    let previous = clipboard.read_content()?;
    clipboard
        .set_text(text.to_string())
        .context("failed to set clipboard text for paste")?;

    let paste_result = send_paste();
    sleep(CLIPBOARD_RESTORE_DELAY);
    let restore_result = restore_clipboard_with_retry(clipboard, previous, &mut sleep);

    paste_result?;
    restore_result?;
    Ok(())
}

fn restore_clipboard_with_retry<C, S>(
    clipboard: &mut C,
    content: Option<ClipboardContent>,
    sleep: &mut S,
) -> Result<()>
where
    C: PasteClipboard,
    S: FnMut(Duration),
{
    let mut last_error = None;
    for attempt in 0..CLIPBOARD_RESTORE_ATTEMPTS {
        match clipboard.set_content(content.clone()) {
            Ok(()) => return Ok(()),
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < CLIPBOARD_RESTORE_ATTEMPTS {
                    sleep(CLIPBOARD_RESTORE_RETRY_DELAY);
                }
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| anyhow::anyhow!("failed to restore clipboard"))
        .context("failed to restore clipboard after paste"))
}

#[cfg(windows)]
struct SystemPasteClipboard {
    inner: arboard::Clipboard,
}

#[cfg(windows)]
impl SystemPasteClipboard {
    fn new() -> Result<Self> {
        Ok(Self {
            inner: arboard::Clipboard::new().context("failed to open Windows clipboard")?,
        })
    }
}

#[cfg(windows)]
impl PasteClipboard for SystemPasteClipboard {
    fn read_content(&mut self) -> Result<Option<ClipboardContent>> {
        if let Ok(image) = self.inner.get_image() {
            return Ok(Some(ClipboardContent::Image(
                crate::clipboard::ClipboardImage {
                    width: image.width,
                    height: image.height,
                    rgba: image.bytes.into_owned(),
                },
            )));
        }

        if let Ok(text) = self.inner.get_text() {
            return Ok(Some(ClipboardContent::Text(text)));
        }

        Ok(None)
    }

    fn set_text(&mut self, text: String) -> Result<()> {
        self.inner
            .set_text(text)
            .context("failed to set clipboard text")
    }

    fn set_content(&mut self, content: Option<ClipboardContent>) -> Result<()> {
        use arboard::ImageData;
        use std::borrow::Cow;

        match content {
            Some(ClipboardContent::Image(image)) => self
                .inner
                .set_image(ImageData {
                    width: image.width,
                    height: image.height,
                    bytes: Cow::Owned(image.rgba),
                })
                .context("failed to restore clipboard image"),
            Some(ClipboardContent::Text(text)) => self
                .inner
                .set_text(text)
                .context("failed to restore clipboard text"),
            Some(ClipboardContent::Empty) | None => self
                .inner
                .clear()
                .context("failed to restore empty clipboard"),
        }
    }
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
    struct FakeRunner;

    impl CommandRunner for FakeRunner {
        fn run(&mut self, _program: &str, _args: &[String]) -> Result<CommandResult> {
            Ok(CommandResult {
                success: true,
                code: Some(0),
                stderr: String::new(),
            })
        }
    }

    struct FakePasteClipboard {
        content: Option<ClipboardContent>,
        fail_image_restores: usize,
    }

    impl PasteClipboard for FakePasteClipboard {
        fn read_content(&mut self) -> Result<Option<ClipboardContent>> {
            Ok(self.content.clone())
        }

        fn set_text(&mut self, text: String) -> Result<()> {
            self.content = Some(ClipboardContent::Text(text));
            Ok(())
        }

        fn set_content(&mut self, content: Option<ClipboardContent>) -> Result<()> {
            if matches!(content, Some(ClipboardContent::Image(_))) && self.fail_image_restores > 0 {
                self.fail_image_restores -= 1;
                anyhow::bail!("clipboard busy");
            }
            self.content = content;
            Ok(())
        }
    }

    #[test]
    fn text_clipboard_on_image_shortcut_is_not_handled() {
        let config = AppConfig::load(None, Default::default()).unwrap();
        let invocation = SshInvocation::parse(vec!["host".to_string()]).unwrap();
        let mut clipboard = FakeClipboard {
            content: ClipboardContent::Text("line1\r\nline2".to_string()),
        };
        let mut uploader = Uploader::new(FakeRunner);

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
        let mut uploader = Uploader::new(FakeRunner);

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
        let mut uploader = Uploader::new(FakeRunner);

        let path = upload_clipboard_image_path(&config, &invocation, &mut clipboard, &mut uploader)
            .unwrap()
            .unwrap();

        assert!(path.starts_with("~/Pictures/paste-ssh/"));
        assert!(path.ends_with(".png"));
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
    fn paste_text_restores_image_clipboard_after_transient_restore_failure() {
        let original = ClipboardContent::Image(ClipboardImage {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
        });
        let mut clipboard = FakePasteClipboard {
            content: Some(original.clone()),
            fail_image_restores: 1,
        };
        let mut paste_sent = false;
        let mut sleeps = Vec::new();

        paste_text_via_clipboard_with(
            &mut clipboard,
            "/tmp/uploaded.png",
            || {
                paste_sent = true;
                Ok(())
            },
            |duration| sleeps.push(duration),
        )
        .unwrap();

        assert!(paste_sent);
        assert_eq!(clipboard.content, Some(original));
        assert_eq!(clipboard.fail_image_restores, 0);
        assert!(sleeps.len() >= 2);
    }
}
