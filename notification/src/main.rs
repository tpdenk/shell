//! Notification daemon.

mod center;

use anyhow::Result;
use config::File;
use serde::{Deserialize, Serialize};
use wayland::{Anchor, KeyboardInteractivity, Layer, LayerConfig, Margin};

use crate::center::Center;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Surface size in logical pixels, anchored to the top-right corner.
    pub width: u32,
    pub height: u32,
    /// Distance from the screen edges, logical pixels.
    pub margin: i32,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            width: 380,
            height: 96,
            margin: 8,
        }
    }
}

impl File for Config {
    const NAME: &'static str = "notification";
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = Config::load()?;

    let layer = LayerConfig {
        namespace: "shell-notification".to_owned(),
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::RIGHT,
        size: (config.width, config.height),
        margin: Margin::same(config.margin),
        exclusive_zone: 0,
        keyboard: KeyboardInteractivity::None,
    };
    wayland::run(layer, |ctx| {
        ui::install(ctx);
        Box::new(Center::new(ctx.clone()))
    })
}
