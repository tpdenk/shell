//! Launcher UI: a query field and the list of matches. Exits once the user
//! picks an entry or dismisses it.

use egui::{RichText, Ui};
use wayland::App;

pub struct Launcher {
    query: String,
    done: bool,
}

impl Launcher {
    pub fn new() -> Launcher {
        Launcher {
            query: String::new(),
            done: false,
        }
    }
}

impl App for Launcher {
    fn ui(&mut self, ui: &mut Ui) {
        let theme = ui::theme(ui.ctx());
        ui::panel(&theme).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.vertical(|ui| {
                let field = egui::TextEdit::singleline(&mut self.query)
                    .hint_text("Search applications")
                    .desired_width(f32::INFINITY);
                let response = ui.add(field);
                response.request_focus();
                ui.separator();
                ui.label(RichText::new("No entries yet").color(theme.subtext));
            });
        });
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.done = true;
        }
    }

    fn wants_exit(&self) -> bool {
        self.done
    }
}
