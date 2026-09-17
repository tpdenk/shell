//! `getoption` reply parsing.
//!
//! Replies look like `{"option": "decoration:rounding", "int": 0, "set": true}`.
//! The value key names the option's type. Observed keys: `int`, `float`,
//! `str`, `css` (`"2 2 2 2"`), `gradient` (`"ff7fbbb3 aa595959 45deg"`).
//! Color options are reported as `int` holding `0xAARRGGBB`.

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::appearance::Rgba;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OptionValue {
    Int(i64),
    Float(f64),
    Str(String),
    /// CSS-style shorthand, `top right bottom left`, already split.
    Css(Vec<i64>),
    /// Gradient color stops. The angle is dropped.
    Gradient(Vec<Rgba>),
}

impl OptionValue {
    pub(crate) fn parse(reply: &str) -> Result<OptionValue> {
        let value: Value =
            serde_json::from_str(reply).with_context(|| format!("invalid JSON: {reply:?}"))?;
        let object = value.as_object().context("reply is not an object")?;
        if let Some(v) = object.get("int") {
            return Ok(OptionValue::Int(
                v.as_i64().context("`int` is not an integer")?,
            ));
        }
        if let Some(v) = object.get("float") {
            return Ok(OptionValue::Float(
                v.as_f64().context("`float` is not a number")?,
            ));
        }
        if let Some(v) = object.get("str") {
            return Ok(OptionValue::Str(
                v.as_str().context("`str` is not a string")?.to_owned(),
            ));
        }
        if let Some(v) = object.get("css") {
            let text = v.as_str().context("`css` is not a string")?;
            let parts = text
                .split_whitespace()
                .map(|p| {
                    p.parse::<i64>()
                        .with_context(|| format!("bad css value {p:?}"))
                })
                .collect::<Result<Vec<_>>>()?;
            return Ok(OptionValue::Css(parts));
        }
        if let Some(v) = object.get("gradient") {
            let text = v.as_str().context("`gradient` is not a string")?;
            let stops = text
                .split_whitespace()
                .filter(|p| !p.ends_with("deg"))
                .map(Rgba::parse_hex_argb)
                .collect::<Result<Vec<_>>>()?;
            return Ok(OptionValue::Gradient(stops));
        }
        bail!("unrecognized option reply: {reply:?}")
    }

    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            OptionValue::Float(v) => Some(*v),
            OptionValue::Int(v) => Some(*v as f64),
            _ => None,
        }
    }

    /// Interprets an `int` option as an `0xAARRGGBB` color.
    pub(crate) fn as_color(&self) -> Option<Rgba> {
        match self {
            OptionValue::Int(v) => u32::try_from(*v).ok().map(Rgba::from_argb),
            _ => None,
        }
    }

    /// First component of a `css` shorthand, or the plain integer.
    pub(crate) fn as_css_first(&self) -> Option<i64> {
        match self {
            OptionValue::Css(parts) => parts.first().copied(),
            OptionValue::Int(v) => Some(*v),
            _ => None,
        }
    }

    pub(crate) fn as_gradient(&self) -> Option<&[Rgba]> {
        match self {
            OptionValue::Gradient(stops) => Some(stops),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_observed_reply_shapes() {
        let int =
            OptionValue::parse(r#"{"option": "decoration:rounding", "int": 12, "set": true }"#)
                .unwrap();
        assert_eq!(int, OptionValue::Int(12));

        let float = OptionValue::parse(
            r#"{"option": "decoration:active_opacity", "float": 0.900000, "set": true }"#,
        )
        .unwrap();
        assert_eq!(float, OptionValue::Float(0.9));

        let css =
            OptionValue::parse(r#"{"option": "general:gaps_out", "css": "4 8 4 8", "set": true }"#)
                .unwrap();
        assert_eq!(css.as_css_first(), Some(4));

        let gradient = OptionValue::parse(
            r#"{"option": "general:col.active_border", "gradient": "ff7fbbb3 aa595959 45deg", "set": true }"#,
        )
        .unwrap();
        assert_eq!(
            gradient.as_gradient().unwrap(),
            &[
                Rgba {
                    r: 0x7f,
                    g: 0xbb,
                    b: 0xb3,
                    a: 0xff
                },
                Rgba {
                    r: 0x59,
                    g: 0x59,
                    b: 0x59,
                    a: 0xaa
                }
            ]
        );

        let color = OptionValue::parse(
            r#"{"option": "misc:background_color", "int": 4279308561, "set": false }"#,
        )
        .unwrap();
        assert_eq!(
            color.as_color(),
            Some(Rgba {
                r: 0x11,
                g: 0x11,
                b: 0x11,
                a: 0xff
            })
        );
    }

    #[test]
    fn rejects_unknown_shapes() {
        assert!(OptionValue::parse(r#"{"option": "x", "set": false }"#).is_err());
        assert!(OptionValue::parse("no such option").is_err());
    }
}
