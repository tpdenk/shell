//! Application launcher.

mod entries;
mod launcher;

use anyhow::Result;
use config::File;
use serde::{Deserialize, Serialize};
use wayland::{Anchor, KeyboardInteractivity, Layer, LayerConfig, Margin};

use crate::entries::Catalog;
use crate::launcher::Launcher;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Surface size in logical pixels, centered on the output.
    pub width: u32,
    pub height: u32,
    /// Command that runs `Terminal=true` entries. The application's command
    /// line is appended.
    pub terminal: String,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            width: 640,
            height: 400,
            terminal: "xdg-terminal-exec".to_owned(),
        }
    }
}

impl File for Config {
    const NAME: &'static str = "launcher";
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let config = Config::load()?;

    let layer = LayerConfig {
        namespace: "shell-launcher".to_owned(),
        layer: Layer::Overlay,
        anchor: Anchor::empty(),
        size: (config.width, config.height),
        margin: Margin::default(),
        exclusive_zone: -1,
        keyboard: KeyboardInteractivity::Exclusive,
    };
    wayland::run(layer, |ctx| {
        ui::install(ctx);
        Box::new(Launcher::new(Catalog::load(config.terminal)))
    })
}
