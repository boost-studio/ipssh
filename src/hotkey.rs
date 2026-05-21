use anyhow::{bail, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub key: char,
    pub modifiers: KeyModifiers,
}

impl Hotkey {
    pub fn parse(input: &str) -> Result<Self> {
        let mut modifiers = KeyModifiers::empty();
        let mut key = None;

        for part in input.split('+') {
            let normalized = part.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
                "shift" => modifiers |= KeyModifiers::SHIFT,
                "alt" => modifiers |= KeyModifiers::ALT,
                "" => bail!("empty hotkey segment in {input}"),
                value if value.chars().count() == 1 => key = value.chars().next(),
                other => bail!("unsupported hotkey segment: {other}"),
            }
        }

        let key = key.ok_or_else(|| anyhow::anyhow!("hotkey must include a key"))?;
        Ok(Self { key, modifiers })
    }

    pub fn matches(&self, event: KeyEvent) -> bool {
        if event.kind == KeyEventKind::Release {
            return false;
        }

        let (event_key, is_control_char) = match event.code {
            KeyCode::Char(ch) => normalize_event_char(ch),
            _ => return false,
        };
        let mut event_modifiers = event.modifiers;
        if is_control_char {
            event_modifiers |= KeyModifiers::CONTROL;
        }

        event_key == self.key && event_modifiers == self.modifiers
    }

    fn physical_keys(&self) -> Vec<PhysicalKey> {
        let mut keys = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            keys.push(PhysicalKey::Control);
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            keys.push(PhysicalKey::Shift);
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            keys.push(PhysicalKey::Alt);
        }
        keys.push(PhysicalKey::Character(self.key));
        keys
    }
}

fn normalize_event_char(ch: char) -> (char, bool) {
    let value = ch as u32;
    if (1..=26).contains(&value) {
        ((b'a' + value as u8 - 1) as char, true)
    } else {
        (ch.to_ascii_lowercase(), false)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalKey {
    Control,
    Shift,
    Alt,
    Character(char),
}

pub trait PhysicalKeyState {
    fn is_pressed(&self, key: PhysicalKey) -> bool;
}

#[derive(Debug)]
pub struct PhysicalHotkeyDetector {
    keys: Vec<PhysicalKey>,
    armed: bool,
}

impl PhysicalHotkeyDetector {
    pub fn new(hotkey: &Hotkey) -> Self {
        Self::from_keys(hotkey.physical_keys())
    }

    pub fn from_keys(keys: Vec<PhysicalKey>) -> Self {
        Self { keys, armed: true }
    }

    pub fn poll<S: PhysicalKeyState>(&mut self, state: &S) -> bool {
        let all_keys_down = self.keys.iter().all(|key| state.is_pressed(*key));
        if !all_keys_down {
            self.armed = true;
            return false;
        }

        if self.armed {
            self.armed = false;
            true
        } else {
            false
        }
    }

    pub fn is_down<S: PhysicalKeyState>(&self, state: &S) -> bool {
        self.keys.iter().all(|key| state.is_pressed(*key))
    }
}

#[cfg(windows)]
pub struct WindowsPhysicalKeyState;

#[cfg(windows)]
impl PhysicalKeyState for WindowsPhysicalKeyState {
    fn is_pressed(&self, key: PhysicalKey) -> bool {
        let virtual_key = match key {
            PhysicalKey::Control => {
                windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL as i32
            }
            PhysicalKey::Shift => windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SHIFT as i32,
            PhysicalKey::Alt => windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_MENU as i32,
            PhysicalKey::Character(ch) => ch.to_ascii_uppercase() as i32,
        };

        unsafe {
            windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(virtual_key) < 0
        }
    }
}

#[cfg(not(windows))]
pub struct WindowsPhysicalKeyState;

#[cfg(not(windows))]
impl PhysicalKeyState for WindowsPhysicalKeyState {
    fn is_pressed(&self, _key: PhysicalKey) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ctrl_v() {
        let hotkey = Hotkey::parse("ctrl+v").unwrap();
        assert_eq!(hotkey.key, 'v');
        assert_eq!(hotkey.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn matches_ctrl_shift_v() {
        let hotkey = Hotkey::parse("ctrl+shift+v").unwrap();
        let event = KeyEvent::new(
            KeyCode::Char('v'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert!(hotkey.matches(event));
    }

    #[test]
    fn matches_ctrl_v_when_windows_reports_control_character() {
        let hotkey = Hotkey::parse("ctrl+v").unwrap();
        let event = KeyEvent::new(KeyCode::Char('\x16'), KeyModifiers::empty());
        assert!(hotkey.matches(event));
    }

    #[test]
    fn matches_ctrl_v_when_windows_reports_control_character_with_control_modifier() {
        let hotkey = Hotkey::parse("ctrl+v").unwrap();
        let event = KeyEvent::new(KeyCode::Char('\x16'), KeyModifiers::CONTROL);
        assert!(hotkey.matches(event));
    }

    #[test]
    fn ignores_hotkey_release_event() {
        let hotkey = Hotkey::parse("ctrl+v").unwrap();
        let event = KeyEvent {
            code: KeyCode::Char('v'),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Release,
            state: crossterm::event::KeyEventState::empty(),
        };
        assert!(!hotkey.matches(event));
    }

    #[test]
    fn rejects_unknown_segment() {
        assert!(Hotkey::parse("meta+v").is_err());
    }

    #[derive(Default)]
    struct FakePhysicalKeys {
        pressed: Vec<PhysicalKey>,
    }

    impl PhysicalKeyState for FakePhysicalKeys {
        fn is_pressed(&self, key: PhysicalKey) -> bool {
            self.pressed.contains(&key)
        }
    }

    #[test]
    fn physical_detector_triggers_ctrl_v_when_configured_keys_are_down() {
        let hotkey = Hotkey::parse("ctrl+v").unwrap();
        let mut detector = PhysicalHotkeyDetector::new(&hotkey);
        let keys = FakePhysicalKeys {
            pressed: vec![PhysicalKey::Control, PhysicalKey::Character('v')],
        };

        assert!(detector.poll(&keys));
    }

    #[test]
    fn physical_detector_does_not_trigger_until_full_sequence_is_down() {
        let hotkey = Hotkey::parse("ctrl+v").unwrap();
        let mut detector = PhysicalHotkeyDetector::new(&hotkey);
        let keys = FakePhysicalKeys {
            pressed: vec![PhysicalKey::Control],
        };

        assert!(!detector.poll(&keys));
    }

    #[test]
    fn physical_detector_does_not_repeat_while_hotkey_stays_down() {
        let hotkey = Hotkey::parse("ctrl+v").unwrap();
        let mut detector = PhysicalHotkeyDetector::new(&hotkey);
        let keys = FakePhysicalKeys {
            pressed: vec![PhysicalKey::Control, PhysicalKey::Character('v')],
        };

        assert!(detector.poll(&keys));
        assert!(!detector.poll(&keys));
    }

    #[test]
    fn physical_detector_rearms_after_release() {
        let hotkey = Hotkey::parse("ctrl+v").unwrap();
        let mut detector = PhysicalHotkeyDetector::new(&hotkey);
        let pressed = FakePhysicalKeys {
            pressed: vec![PhysicalKey::Control, PhysicalKey::Character('v')],
        };
        let released = FakePhysicalKeys::default();

        assert!(detector.poll(&pressed));
        assert!(!detector.poll(&released));
        assert!(detector.poll(&pressed));
    }
}
