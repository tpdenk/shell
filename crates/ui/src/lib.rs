//! Look shared by every shell component.
//!
//! The [`Theme`] is derived from the running Hyprland instance (rounding,
//! border size, gaps, border colors, background, opacity) and kept in the
//! [`egui::Context`]. It follows `hyprctl reload` live. Without Hyprland the
//! built-in defaults apply.

mod appearance;
mod option;
mod theme;

use std::sync::Arc;

use egui::{Context, CornerRadius, Frame, Id, Margin};
use hypr::Instance;

use crate::appearance::Appearance;
pub use crate::theme::Theme;

const THEME_ID: &str = "shell.theme";

/// Loads the theme, applies it, and keeps it updated for the lifetime of the
/// process. Call once per component, before the first frame.
pub fn install(ctx: &Context) {
    let instance = match Instance::from_env() {
        Ok(instance) => instance,
        Err(err) => {
            log::info!("using built-in theme: {err}");
            set_theme(ctx, Theme::default());
            return;
        }
    };
    set_theme(ctx, load_theme(&instance));

    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("hypr-events".into())
        .spawn(move || {
            let events = match instance.events() {
                Ok(events) => events,
                Err(err) => {
                    log::warn!("theme will not follow config reloads: {err:#}");
                    return;
                }
            };
            // Ends when Hyprland closes the socket.
            for event in events {
                match event {
                    Ok(event) if event.name == "configreloaded" => {
                        set_theme(&ctx, load_theme(&instance));
                        ctx.request_repaint();
                    }
                    Ok(_) => {}
                    Err(err) => {
                        log::warn!("Hyprland event stream ended: {err}");
                        return;
                    }
                }
            }
        });
    if let Err(err) = spawned {
        log::warn!("theme will not follow config reloads: {err}");
    }
}

fn load_theme(instance: &Instance) -> Theme {
    match Appearance::load(instance) {
        Ok(appearance) => Theme::from_appearance(&appearance),
        Err(err) => {
            log::warn!("using built-in theme: {err:#}");
            Theme::default()
        }
    }
}

/// Stores `theme` in the context and restyles egui accordingly.
pub fn set_theme(ctx: &Context, theme: Theme) {
    theme.apply(ctx);
    ctx.data_mut(|d| d.insert_temp(Id::new(THEME_ID), Arc::new(theme)));
}

/// The current theme. Cheap, safe to call every frame.
pub fn theme(ctx: &Context) -> Arc<Theme> {
    ctx.data(|d| d.get_temp::<Arc<Theme>>(Id::new(THEME_ID)))
        .unwrap_or_default()
}

/// Background frame for a whole layer surface, rounded per the theme.
pub fn panel(theme: &Theme) -> Frame {
    Frame::new()
        .fill(theme.background)
        .corner_radius(CornerRadius::same(theme.rounding))
        .inner_margin(Margin::symmetric(10, 6))
}
