//! Launcher UI: a query field and the ranked list of matches. Exits once the
//! user starts an entry or dismisses it. Everything about desktop entries
//! lives in [`Catalog`]. This file owns only UI state.

use egui::text::LayoutJob;
use egui::{Key, Modifiers, RichText, ScrollArea, Style, TextEdit, TextStyle, Ui};
use ui::Theme;
use wayland::App;

use crate::entries::{Catalog, Entry};

pub struct Launcher {
    catalog: Catalog,
    query: String,
    /// Indices into the catalog, current ranking for `query`.
    results: Vec<usize>,
    /// Row in `results`.
    selected: usize,
    /// Scroll the selected row into view this frame. Set by keyboard
    /// navigation and query changes.
    reveal: bool,
    error: Option<String>,
    done: bool,
}

impl Launcher {
    pub fn new(mut catalog: Catalog) -> Launcher {
        let results = catalog.search("");
        Launcher {
            catalog,
            query: String::new(),
            results,
            selected: 0,
            reveal: false,
            error: None,
            done: false,
        }
    }

    /// Consumes navigation keys before the text field can see them. Tab is
    /// deliberately unbound, egui's focus cycling owns it.
    fn handle_keys(&mut self, ui: &Ui) {
        let pressed = |modifiers, key| ui.input_mut(|i| i.consume_key(modifiers, key));
        if pressed(Modifiers::NONE, Key::Escape) {
            self.done = true;
        }
        if pressed(Modifiers::NONE, Key::ArrowDown) || pressed(Modifiers::COMMAND, Key::N) {
            self.select(self.selected + 1);
        }
        if pressed(Modifiers::NONE, Key::ArrowUp) || pressed(Modifiers::COMMAND, Key::P) {
            self.select(self.selected.saturating_sub(1));
        }
        if pressed(Modifiers::NONE, Key::Enter) {
            self.launch(self.selected);
        }
    }

    /// Moves the highlight, clamped to the results. No wrap-around.
    fn select(&mut self, row: usize) {
        self.selected = row.min(self.results.len().saturating_sub(1));
        self.reveal = true;
    }

    fn show_query(&mut self, ui: &mut Ui) {
        let field = TextEdit::singleline(&mut self.query)
            .hint_text("Search applications")
            .desired_width(f32::INFINITY)
            .font(TextStyle::Heading)
            .frame(egui::Frame::NONE);
        let response = ui.add(field);
        response.request_focus();
        if response.changed() {
            self.results = self.catalog.search(&self.query);
            self.select(0);
        }
    }

    fn show_results(&mut self, ui: &mut Ui, theme: &Theme) {
        let mut clicked = None;
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            if self.results.is_empty() {
                ui.label(RichText::new("No matches").color(theme.subtext));
                return;
            }
            for (row, &index) in self.results.iter().enumerate() {
                let highlighted = row == self.selected;
                let text = row_text(self.catalog.entry(index), ui.style(), theme);
                let response = ui.selectable_label(highlighted, text);
                if highlighted && self.reveal {
                    response.scroll_to_me(None);
                }
                if response.clicked() {
                    clicked = Some(row);
                }
            }
        });
        self.reveal = false;
        if let Some(row) = clicked {
            self.launch(row);
        }
    }

    fn launch(&mut self, row: usize) {
        let Some(&index) = self.results.get(row) else {
            return;
        };
        match self.catalog.launch(index) {
            Ok(()) => self.done = true,
            Err(err) => self.error = Some(format!("{err:#}")),
        }
    }
}

/// Name in body text, generic name (if any) trailing in small subtext.
fn row_text(entry: &Entry, style: &Style, theme: &Theme) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.append(&entry.name, 0.0, egui::TextFormat {
        font_id: TextStyle::Body.resolve(style),
        color: theme.text,
        ..Default::default()
    });
    if let Some(generic) = &entry.generic_name {
        job.append(generic, 12.0, egui::TextFormat {
            font_id: TextStyle::Small.resolve(style),
            color: theme.subtext,
            ..Default::default()
        });
    }
    job
}

impl App for Launcher {
    fn ui(&mut self, ui: &mut Ui) {
        let theme = ui::theme(ui.ctx());
        self.handle_keys(ui);
        if self.done {
            return;
        }
        ui::panel(&theme).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            self.show_query(ui);
            ui.separator();
            if let Some(err) = &self.error {
                ui.label(RichText::new(err).color(theme.error));
            }
            self.show_results(ui, &theme);
        });
    }

    fn wants_exit(&self) -> bool {
        self.done
    }
}
