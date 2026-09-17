//! Theme model and its egui styling.

use egui::{Color32, Context, Stroke, Visuals};

use crate::appearance::{Appearance, Rgba};

/// Resolved colors and metrics, in egui terms.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// Corner radius of panels, logical pixels.
    pub rounding: u8,
    /// Stroke width matching window borders.
    pub border: f32,
    /// Spacing matching `gaps_out`.
    pub gap: f32,
    /// Panel fill, translucency included.
    pub background: Color32,
    /// Slightly raised areas: inactive widgets, separators.
    pub surface: Color32,
    /// Hovered widgets.
    pub surface_hi: Color32,
    /// Pressed widgets.
    pub surface_active: Color32,
    pub text: Color32,
    pub subtext: Color32,
    /// First stop of the active border gradient.
    pub accent: Color32,
    /// Second stop of the active border gradient, or `accent`.
    pub accent_alt: Color32,
    /// First stop of the inactive border gradient.
    pub inactive: Color32,
    pub warn: Color32,
    pub error: Color32,
}

impl Default for Theme {
    /// Catppuccin Mocha, used when no Hyprland instance is reachable.
    fn default() -> Theme {
        Theme {
            rounding: 10,
            border: 2.0,
            gap: 8.0,
            background: Color32::from_rgba_unmultiplied(0x1e, 0x1e, 0x2e, 0xe6),
            surface: Color32::from_rgb(0x31, 0x32, 0x44),
            surface_hi: Color32::from_rgb(0x45, 0x47, 0x5a),
            surface_active: Color32::from_rgb(0x58, 0x5b, 0x70),
            text: Color32::from_rgb(0xcd, 0xd6, 0xf4),
            subtext: Color32::from_rgb(0xa6, 0xad, 0xc8),
            accent: Color32::from_rgb(0x89, 0xb4, 0xfa),
            accent_alt: Color32::from_rgb(0xb4, 0xbe, 0xfe),
            inactive: Color32::from_rgb(0x58, 0x5b, 0x70),
            warn: Color32::from_rgb(0xf9, 0xe2, 0xaf),
            error: Color32::from_rgb(0xf3, 0x8b, 0xa8),
        }
    }
}

impl Theme {
    /// Derives a theme from Hyprland's decoration settings. Hyprland only
    /// knows background and border colors, so text and surface shades are
    /// interpolated between the background and black or white, whichever
    /// contrasts with it.
    pub(crate) fn from_appearance(a: &Appearance) -> Theme {
        let defaults = Theme::default();
        let base = opaque(a.background);
        let fg = if luminance(base) < 0.5 {
            Color32::WHITE
        } else {
            Color32::BLACK
        };
        let alpha = (a.active_opacity * 255.0).round() as u8;
        let background = Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), alpha);
        let active = a.active_border.first().copied().map(opaque);
        let inactive = a.inactive_border.first().copied().map(opaque);
        Theme {
            rounding: u8::try_from(a.rounding).unwrap_or(u8::MAX),
            border: a.border_size as f32,
            gap: a.gaps_out as f32,
            background,
            surface: mix(base, fg, 0.08),
            surface_hi: mix(base, fg, 0.14),
            surface_active: mix(base, fg, 0.22),
            text: mix(base, fg, 0.88),
            subtext: mix(base, fg, 0.60),
            accent: active.unwrap_or(defaults.accent),
            accent_alt: a
                .active_border
                .get(1)
                .copied()
                .map(opaque)
                .or(active)
                .unwrap_or(defaults.accent_alt),
            inactive: inactive.unwrap_or(defaults.inactive),
            warn: defaults.warn,
            error: defaults.error,
        }
    }

    /// Restyles egui's widgets with this theme.
    pub fn apply(&self, ctx: &Context) {
        let dark = luminance(self.background) < 0.5;
        let mut visuals = if dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        visuals.override_text_color = Some(self.text);
        visuals.panel_fill = self.background;
        visuals.window_fill = self.background;
        visuals.extreme_bg_color = mix(self.background, Color32::BLACK, 0.3);
        visuals.faint_bg_color = self.surface;
        visuals.selection.bg_fill = self.accent.linear_multiply(0.4);
        visuals.selection.stroke = Stroke::new(1.0, self.accent);
        visuals.hyperlink_color = self.accent;
        visuals.warn_fg_color = self.warn;
        visuals.error_fg_color = self.error;
        visuals.window_corner_radius = egui::CornerRadius::same(self.rounding);
        visuals.menu_corner_radius = egui::CornerRadius::same(self.rounding);

        let widgets = &mut visuals.widgets;
        widgets.noninteractive.bg_fill = self.surface;
        widgets.noninteractive.weak_bg_fill = self.surface;
        widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.surface_hi);
        widgets.noninteractive.fg_stroke = Stroke::new(1.0, self.subtext);
        widgets.inactive.bg_fill = self.surface;
        widgets.inactive.weak_bg_fill = self.surface;
        widgets.inactive.fg_stroke = Stroke::new(1.0, self.text);
        widgets.hovered.bg_fill = self.surface_hi;
        widgets.hovered.weak_bg_fill = self.surface_hi;
        widgets.hovered.bg_stroke = Stroke::new(1.0, self.accent);
        widgets.hovered.fg_stroke = Stroke::new(1.5, self.text);
        widgets.active.bg_fill = self.surface_active;
        widgets.active.weak_bg_fill = self.surface_active;
        widgets.active.bg_stroke = Stroke::new(1.0, self.accent);
        widgets.active.fg_stroke = Stroke::new(2.0, self.text);
        let radius = egui::CornerRadius::same(self.rounding.min(6));
        for w in [
            &mut widgets.noninteractive,
            &mut widgets.inactive,
            &mut widgets.hovered,
            &mut widgets.active,
            &mut widgets.open,
        ] {
            w.corner_radius = radius;
        }
        ctx.set_visuals(visuals);
    }
}

fn opaque(c: Rgba) -> Color32 {
    Color32::from_rgb(c.r, c.g, c.b)
}

/// Relative luminance of the color as it would look at full opacity, in
/// `0.0..=1.0`. `Color32` stores premultiplied channels, so unmultiply first.
fn luminance(c: Color32) -> f32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    let channel = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)
}

/// Linear interpolation of opaque colors in sRGB space, `t` toward `to`.
fn mix(from: Color32, to: Color32, t: f32) -> Color32 {
    let lerp = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(
        lerp(from.r(), to.r()),
        lerp(from.g(), to.g()),
        lerp(from.b(), to.b()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appearance(background: Rgba) -> Appearance {
        Appearance {
            rounding: 12,
            border_size: 3,
            gaps_in: 2,
            gaps_out: 6,
            active_border: vec![Rgba {
                r: 0x7f,
                g: 0xbb,
                b: 0xb3,
                a: 0xff,
            }],
            inactive_border: vec![Rgba {
                r: 0x59,
                g: 0x59,
                b: 0x59,
                a: 0xaa,
            }],
            background,
            active_opacity: 0.9,
        }
    }

    #[test]
    fn dark_background_gets_light_text_and_vice_versa() {
        let dark = Theme::from_appearance(&appearance(Rgba {
            r: 0x11,
            g: 0x11,
            b: 0x11,
            a: 0xff,
        }));
        assert!(luminance(dark.text) > 0.7, "{:?}", dark.text);
        assert!(luminance(dark.surface) > luminance(dark.background));

        let light = Theme::from_appearance(&appearance(Rgba {
            r: 0xf0,
            g: 0xf0,
            b: 0xf0,
            a: 0xff,
        }));
        assert!(luminance(light.text) < 0.1, "{:?}", light.text);
        assert!(luminance(light.surface) < luminance(light.background));
    }

    #[test]
    fn metrics_and_borders_come_from_hyprland() {
        let a = appearance(Rgba {
            r: 0x11,
            g: 0x11,
            b: 0x11,
            a: 0xff,
        });
        let theme = Theme::from_appearance(&a);
        assert_eq!(theme.rounding, 12);
        assert_eq!(theme.border, 3.0);
        assert_eq!(theme.gap, 6.0);
        assert_eq!(theme.accent, Color32::from_rgb(0x7f, 0xbb, 0xb3));
        assert_eq!(theme.accent_alt, theme.accent);
        assert_eq!(theme.inactive, Color32::from_rgb(0x59, 0x59, 0x59));
        assert_eq!(theme.background.a(), 230);
    }
}
