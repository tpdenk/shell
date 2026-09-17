//! The subset of Hyprland's configuration that describes how things look,
//! read with `getoption` over the request socket. Hyprland reports gaps as
//! `css` and border colors as `gradient` values, parsed by [`OptionValue`].

use anyhow::{Context, Result, bail};
use hypr::Instance;

use crate::option::OptionValue;

/// Straight-alpha sRGB color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub(crate) const fn from_argb(argb: u32) -> Rgba {
        Rgba {
            a: (argb >> 24) as u8,
            r: (argb >> 16) as u8,
            g: (argb >> 8) as u8,
            b: argb as u8,
        }
    }

    /// Parses the `AARRGGBB` form Hyprland uses inside gradients.
    pub(crate) fn parse_hex_argb(text: &str) -> Result<Rgba> {
        let digits = text.trim_start_matches("0x");
        let value =
            u32::from_str_radix(digits, 16).with_context(|| format!("bad color {text:?}"))?;
        Ok(match digits.len() {
            6 => Rgba::from_argb(0xff00_0000 | value),
            8 => Rgba::from_argb(value),
            _ => bail!("bad color length {text:?}"),
        })
    }
}

/// Decoration and color settings, in Hyprland's own terms.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Appearance {
    /// `decoration:rounding`, logical pixels.
    pub rounding: u32,
    /// `general:border_size`, logical pixels.
    pub border_size: u32,
    /// `general:gaps_in`, first component.
    pub gaps_in: u32,
    /// `general:gaps_out`, first component.
    pub gaps_out: u32,
    /// `general:col.active_border` stops, at least one.
    pub active_border: Vec<Rgba>,
    /// `general:col.inactive_border` stops, at least one.
    pub inactive_border: Vec<Rgba>,
    /// `misc:background_color`.
    pub background: Rgba,
    /// `decoration:active_opacity`, `0.0..=1.0`.
    pub active_opacity: f32,
}

impl Appearance {
    /// Reads the current settings from `instance`.
    pub(crate) fn load(instance: &Instance) -> Result<Appearance> {
        let option = |name: &str| -> Result<OptionValue> {
            let reply = instance.request(&format!("j/getoption {name}"))?;
            OptionValue::parse(&reply).with_context(|| format!("parsing option {name}"))
        };
        let int = |name: &str| -> Result<u32> {
            let value = option(name)?;
            let raw = value
                .as_css_first()
                .with_context(|| format!("{name} is not an integer: {value:?}"))?;
            Ok(u32::try_from(raw.max(0)).unwrap_or(u32::MAX))
        };
        let gradient = |name: &str| -> Result<Vec<Rgba>> {
            let value = option(name)?;
            let stops = value
                .as_gradient()
                .with_context(|| format!("{name} is not a gradient: {value:?}"))?;
            anyhow::ensure!(!stops.is_empty(), "{name} has no color stops");
            Ok(stops.to_vec())
        };

        let background = option("misc:background_color")?;
        let opacity = option("decoration:active_opacity")?;
        Ok(Appearance {
            rounding: int("decoration:rounding")?,
            border_size: int("general:border_size")?,
            gaps_in: int("general:gaps_in")?,
            gaps_out: int("general:gaps_out")?,
            active_border: gradient("general:col.active_border")?,
            inactive_border: gradient("general:col.inactive_border")?,
            background: background
                .as_color()
                .with_context(|| format!("background_color is not a color: {background:?}"))?,
            active_opacity: opacity
                .as_f64()
                .with_context(|| format!("active_opacity is not a number: {opacity:?}"))?
                .clamp(0.0, 1.0) as f32,
        })
    }
}
