//! Translation between Wayland input events and egui's input model.

use egui::{Event, Modifiers, MouseWheelUnit, Pos2, TouchPhase, Vec2};
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

    pub(crate) fn take(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
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
