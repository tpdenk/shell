//! Bar UI. Pure egui: it never touches Wayland or GPU types. Widgets
//! (workspaces, clock, tray) will live here.

use egui::{Context, Margin, Ui};
use wayland::App;

use crate::Config;
use crate::bar::workspace::WorkspacesWidget;

mod workspace;

pub struct Bar;

impl Bar {
    pub fn new(ctx: &Context, config: Config) -> Bar {
        workspace::install(ctx, config.workspaces);
        Bar
    }
}

impl App for Bar {
    fn ui(&mut self, ui: &mut Ui) {
        let theme = ui::theme(ui.ctx());
        // A full-width bar flush with the screen edge has no corners to round.
        // No vertical margin: the row must be at least `interact_size.y` tall
        // for its widgets to center instead of overflowing downward.
        ui::panel(&theme)
            .corner_radius(0)
            .inner_margin(Margin::symmetric(10, 0))
            .show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                ui.horizontal_centered(|ui| {
                    ui.add(WorkspacesWidget);
                });
            });
    }
}
