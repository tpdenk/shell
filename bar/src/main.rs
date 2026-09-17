//! Top bar.

mod bar;

use anyhow::Result;
use config::File;
use serde::{Deserialize, Serialize};
use wayland::{Anchor, KeyboardInteractivity, Layer, LayerConfig, Margin};

use crate::bar::Bar;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Height in logical pixels. Also the exclusive zone.
    pub height: u32,
    pub workspaces: WorkspacesConfig,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            height: 24,
            workspaces: WorkspacesConfig::default(),
        }
    }
}

impl File for Config {
    const NAME: &'static str = "bar";
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WorkspacesConfig {
    /// Show workspaces without windows. Hyprland only reports empty
    /// workspaces that are persistent or currently focused.
    pub show_empty: bool,
    /// Show special (scratchpad) workspaces.
    pub show_special: bool,
    /// Gap between workspace buttons, logical pixels.
    pub spacing: f32,
    /// Corner radius cap for the buttons. The theme's rounding applies up
    /// to this value.
    pub max_rounding: u8,
    /// Outline width of workspaces shown on other monitors.
    pub visible_stroke: f32,
}

impl Default for WorkspacesConfig {
    fn default() -> WorkspacesConfig {
        WorkspacesConfig {
            show_empty: true,
            show_special: false,
            spacing: 4.0,
            max_rounding: 6,
            visible_stroke: 1.0,
        }
    }
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = Config::load()?;

    let layer = LayerConfig {
        namespace: "shell-bar".to_owned(),
        layer: Layer::Top,
        anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
        size: (0, config.height),
        margin: Margin::default(),
        exclusive_zone: config.height as i32,
        keyboard: KeyboardInteractivity::None,
    };
    wayland::run(layer, |ctx| {
        ui::install(ctx);
        Box::new(Bar::new(ctx, config))
    })
}
