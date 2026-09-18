//! Translation between Wayland input events and egui's input model.

use egui::{Event, Key, Modifiers, MouseWheelUnit, Pos2, TouchPhase, Vec2};
use smithay_client_toolkit::seat::keyboard::{KeyEvent, Keysym, Modifiers as KeyModifiers};
use smithay_client_toolkit::seat::pointer::{CursorIcon, PointerEvent, PointerEventKind};

/// Linux `input-event-codes.h` button values delivered by `wl_pointer`.
const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

/// Accumulates egui events between frames.
#[derive(Default)]
pub(crate) struct InputState {
    pub(crate) events: Vec<Event>,
    pub(crate) modifiers: Modifiers,
    /// Pointer is currently over the surface.
    pub(crate) pointer_inside: bool,
}

impl InputState {
    /// Appends the egui equivalents of `event`. Returns `true` if egui may
    /// need to repaint because of it.
    pub(crate) fn push_pointer(&mut self, event: &PointerEvent) -> bool {
        let pos = Pos2::new(event.position.0 as f32, event.position.1 as f32);
        match event.kind {
            PointerEventKind::Enter { .. } => {
                self.pointer_inside = true;
                self.events.push(Event::PointerMoved(pos));
            }
            PointerEventKind::Leave { .. } => {
                self.pointer_inside = false;
                self.events.push(Event::PointerGone);
            }
            PointerEventKind::Motion { .. } => self.events.push(Event::PointerMoved(pos)),
            PointerEventKind::Press { button, .. } | PointerEventKind::Release { button, .. } => {
                let Some(button) = pointer_button(button) else {
                    return false;
                };
                let pressed = matches!(event.kind, PointerEventKind::Press { .. });
                self.events.push(Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: self.modifiers,
                });
            }
            PointerEventKind::Axis {
                horizontal,
                vertical,
                ..
            } => {
                // Wayland reports how far the *finger* moved. egui wants how
                // far the *content* moves, which is the opposite direction.
                let (unit, delta) = if horizontal.value120 != 0 || vertical.value120 != 0 {
                    let steps = |v: i32| -(v as f32) / 120.0;
                    (
                        MouseWheelUnit::Line,
                        Vec2::new(steps(horizontal.value120), steps(vertical.value120)),
                    )
                } else if horizontal.discrete != 0 || vertical.discrete != 0 {
                    (
                        MouseWheelUnit::Line,
                        Vec2::new(-(horizontal.discrete as f32), -(vertical.discrete as f32)),
                    )
                } else {
                    (
                        MouseWheelUnit::Point,
                        Vec2::new(-(horizontal.absolute as f32), -(vertical.absolute as f32)),
                    )
                };
                if delta == Vec2::ZERO {
                    return false;
                }
                let phase = if horizontal.stop || vertical.stop {
                    TouchPhase::End
                } else {
                    TouchPhase::Move
                };
                self.events.push(Event::MouseWheel {
                    unit,
                    delta,
                    phase,
                    modifiers: self.modifiers,
                });
            }
        }
        true
    }

    /// Replaces the modifier state and tells egui about it.
    pub(crate) fn set_modifiers(&mut self, m: KeyModifiers) {
        self.modifiers = Modifiers {
            alt: m.alt,
            ctrl: m.ctrl,
            shift: m.shift,
            mac_cmd: false,
            command: m.ctrl,
        };
        self.events.push(Event::ModifiersChanged(self.modifiers));
    }

    /// Appends the egui equivalents of a key press, repeat, or release.
    pub(crate) fn push_key(&mut self, event: &KeyEvent, pressed: bool) {
        if let Some(key) = egui_key(event.keysym) {
            self.events.push(Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: self.modifiers,
            });
        }
        // Text only on press, never for control characters (Return arrives
        // as "\r"), and never while Ctrl is held: Ctrl+A is a command, not
        // the letter.
        if pressed
            && !self.modifiers.ctrl
            && let Some(text) = &event.utf8
            && !text.is_empty()
            && text.chars().all(is_printable_char)
        {
            self.events.push(Event::Text(text.clone()));
        }
    }

    pub(crate) fn take(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
}

/// Characters egui should insert as text. Excludes ASCII control characters
/// and the Unicode private-use areas, which keyboards use for dead keys and
/// vendor keys.
fn is_printable_char(c: char) -> bool {
    let private_use = matches!(
        c,
        '\u{e000}'..='\u{f8ff}' | '\u{f0000}'..='\u{ffffd}' | '\u{100000}'..='\u{10fffd}'
    );
    !c.is_ascii_control() && !private_use
}

fn egui_key(keysym: Keysym) -> Option<Key> {
    Some(match keysym {
        Keysym::Escape => Key::Escape,
        Keysym::Tab | Keysym::ISO_Left_Tab => Key::Tab,
        Keysym::BackSpace => Key::Backspace,
        Keysym::Return | Keysym::KP_Enter => Key::Enter,
        Keysym::Insert | Keysym::KP_Insert => Key::Insert,
        Keysym::Delete | Keysym::KP_Delete => Key::Delete,
        Keysym::Home | Keysym::KP_Home => Key::Home,
        Keysym::End | Keysym::KP_End => Key::End,
        Keysym::Page_Up | Keysym::KP_Page_Up => Key::PageUp,
        Keysym::Page_Down | Keysym::KP_Page_Down => Key::PageDown,
        Keysym::Up | Keysym::KP_Up => Key::ArrowUp,
        Keysym::Down | Keysym::KP_Down => Key::ArrowDown,
        Keysym::Left | Keysym::KP_Left => Key::ArrowLeft,
        Keysym::Right | Keysym::KP_Right => Key::ArrowRight,
        Keysym::F1 => Key::F1,
        Keysym::F2 => Key::F2,
        Keysym::F3 => Key::F3,
        Keysym::F4 => Key::F4,
        Keysym::F5 => Key::F5,
        Keysym::F6 => Key::F6,
        Keysym::F7 => Key::F7,
        Keysym::F8 => Key::F8,
        Keysym::F9 => Key::F9,
        Keysym::F10 => Key::F10,
        Keysym::F11 => Key::F11,
        Keysym::F12 => Key::F12,
        Keysym::Shift_L => Key::ShiftLeft,
        Keysym::Shift_R => Key::ShiftRight,
        Keysym::Control_L => Key::ControlLeft,
        Keysym::Control_R => Key::ControlRight,
        Keysym::Alt_L => Key::AltLeft,
        Keysym::Alt_R => Key::AltRight,
        Keysym::Super_L => Key::SuperLeft,
        Keysym::Super_R => Key::SuperRight,
        _ => return key_from_char(keysym.key_char()?),
    })
}

fn key_from_char(c: char) -> Option<Key> {
    const DIGITS: [Key; 10] = [
        Key::Num0,
        Key::Num1,
        Key::Num2,
        Key::Num3,
        Key::Num4,
        Key::Num5,
        Key::Num6,
        Key::Num7,
        Key::Num8,
        Key::Num9,
    ];
    Some(match c {
        'a'..='z' | 'A'..='Z' => Key::from_name(c.to_ascii_uppercase().encode_utf8(&mut [0; 4]))?,
        '0'..='9' => DIGITS[(c as u8 - b'0') as usize],
        ' ' => Key::Space,
        ':' => Key::Colon,
        ',' => Key::Comma,
        '\\' => Key::Backslash,
        '/' => Key::Slash,
        '|' => Key::Pipe,
        '?' => Key::Questionmark,
        '!' => Key::Exclamationmark,
        '[' => Key::OpenBracket,
        ']' => Key::CloseBracket,
        '{' => Key::OpenCurlyBracket,
        '}' => Key::CloseCurlyBracket,
        '`' => Key::Backtick,
        '-' => Key::Minus,
        '.' => Key::Period,
        '+' => Key::Plus,
        '=' => Key::Equals,
        ';' => Key::Semicolon,
        '\'' => Key::Quote,
        _ => return None,
    })
}

fn pointer_button(code: u32) -> Option<egui::PointerButton> {
    Some(match code {
        BTN_LEFT => egui::PointerButton::Primary,
        BTN_RIGHT => egui::PointerButton::Secondary,
        BTN_MIDDLE => egui::PointerButton::Middle,
        BTN_SIDE => egui::PointerButton::Extra1,
        BTN_EXTRA => egui::PointerButton::Extra2,
        _ => return None,
    })
}

/// Maps egui's cursor request onto the CSS cursor names Wayland uses.
pub(crate) fn cursor_icon(icon: egui::CursorIcon) -> Option<CursorIcon> {
    use egui::CursorIcon as E;
    Some(match icon {
        E::None => return None,
        E::Default => CursorIcon::Default,
        E::ContextMenu => CursorIcon::ContextMenu,
        E::Help => CursorIcon::Help,
        E::PointingHand => CursorIcon::Pointer,
        E::Progress => CursorIcon::Progress,
        E::Wait => CursorIcon::Wait,
        E::Cell => CursorIcon::Cell,
        E::Crosshair => CursorIcon::Crosshair,
        E::Text => CursorIcon::Text,
        E::VerticalText => CursorIcon::VerticalText,
        E::Alias => CursorIcon::Alias,
        E::Copy => CursorIcon::Copy,
        E::Move => CursorIcon::Move,
        E::NoDrop => CursorIcon::NoDrop,
        E::NotAllowed => CursorIcon::NotAllowed,
        E::Grab => CursorIcon::Grab,
        E::Grabbing => CursorIcon::Grabbing,
        E::AllScroll => CursorIcon::AllScroll,
        E::ResizeHorizontal => CursorIcon::EwResize,
        E::ResizeNeSw => CursorIcon::NeswResize,
        E::ResizeNwSe => CursorIcon::NwseResize,
        E::ResizeVertical => CursorIcon::NsResize,
        E::ResizeEast => CursorIcon::EResize,
        E::ResizeSouthEast => CursorIcon::SeResize,
        E::ResizeSouth => CursorIcon::SResize,
        E::ResizeSouthWest => CursorIcon::SwResize,
        E::ResizeWest => CursorIcon::WResize,
        E::ResizeNorthWest => CursorIcon::NwResize,
        E::ResizeNorth => CursorIcon::NResize,
        E::ResizeNorthEast => CursorIcon::NeResize,
        E::ResizeColumn => CursorIcon::ColResize,
        E::ResizeRow => CursorIcon::RowResize,
        E::ZoomIn => CursorIcon::ZoomIn,
        E::ZoomOut => CursorIcon::ZoomOut,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(keysym: Keysym, utf8: Option<&str>) -> KeyEvent {
        KeyEvent {
            time: 0,
            raw_code: 0,
            keysym,
            utf8: utf8.map(str::to_owned),
        }
    }

    fn press(input: &mut InputState, keysym: Keysym, utf8: Option<&str>) -> Vec<Event> {
        input.push_key(&key_event(keysym, utf8), true);
        input.take()
    }

    #[test]
    fn return_is_enter_without_text() {
        let mut input = InputState::default();
        let events = press(&mut input, Keysym::Return, Some("\r"));
        assert!(
            matches!(
                events.as_slice(),
                [Event::Key {
                    key: Key::Enter,
                    pressed: true,
                    ..
                }]
            ),
            "{events:?}"
        );
    }

    #[test]
    fn keypad_enter_is_enter() {
        let mut input = InputState::default();
        let events = press(&mut input, Keysym::KP_Enter, Some("\r"));
        assert!(matches!(events.as_slice(), [Event::Key { key: Key::Enter, .. }]));
    }

    #[test]
    fn ctrl_letter_is_a_command_not_text() {
        let mut input = InputState::default();
        input.set_modifiers(KeyModifiers {
            ctrl: true,
            ..KeyModifiers::default()
        });
        input.take();
        let events = press(&mut input, Keysym::a, Some("\u{1}"));
        assert!(
            matches!(events.as_slice(), [Event::Key { key: Key::A, .. }]),
            "{events:?}"
        );
    }

    #[test]
    fn shifted_letter_yields_key_and_text() {
        let mut input = InputState::default();
        let events = press(&mut input, Keysym::A, Some("A"));
        assert!(
            matches!(
                events.as_slice(),
                [Event::Key { key: Key::A, .. }, Event::Text(text)] if text == "A"
            ),
            "{events:?}"
        );
    }
}
