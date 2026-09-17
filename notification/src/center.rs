//! Notification list and the surface that shows the newest one.
//!
//! Notifications arrive from other threads (the D-Bus server, once wired)
//! through [`Center::sender`]. The surface is mapped only while the list is
//! non-empty.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use egui::{Align, Layout, RichText, Ui};
use wayland::App;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct Notification {
    pub app_name: String,
    pub summary: String,
    pub body: String,
    /// `None` keeps the notification until it is dismissed.
    pub timeout: Option<Duration>,
}

struct Shown {
    notification: Notification,
    received: Instant,
}

impl Shown {
    fn expires_at(&self) -> Option<Instant> {
        self.notification.timeout.map(|t| self.received + t)
    }
}

pub struct Center {
    ctx: egui::Context,
    incoming: Receiver<Notification>,
    sender: Sender<Notification>,
    queue: Vec<Shown>,
}

impl Center {
    pub fn new(ctx: egui::Context) -> Center {
        let (sender, incoming) = channel();
        Center {
            ctx,
            incoming,
            sender,
            queue: Vec::new(),
        }
    }

    /// Hand this to whatever produces notifications. Sending wakes the UI.
    #[allow(dead_code)]
    pub fn sender(&self) -> NotificationSender {
        NotificationSender {
            tx: self.sender.clone(),
            ctx: self.ctx.clone(),
        }
    }

    /// Pulls new notifications in and drops expired ones.
    fn sync(&mut self) {
        let now = Instant::now();
        while let Ok(mut notification) = self.incoming.try_recv() {
            notification.timeout.get_or_insert(DEFAULT_TIMEOUT);
            self.queue.push(Shown {
                notification,
                received: now,
            });
        }
        self.queue
            .retain(|shown| shown.expires_at().is_none_or(|t| t > now));
    }
}

/// Thread-safe producer end of a [`Center`].
#[derive(Clone)]
pub struct NotificationSender {
    tx: Sender<Notification>,
    ctx: egui::Context,
}

impl NotificationSender {
    #[allow(dead_code)]
    pub fn send(&self, notification: Notification) {
        if self.tx.send(notification).is_ok() {
            self.ctx.request_repaint();
        }
    }
}

impl App for Center {
    fn ui(&mut self, ui: &mut Ui) {
        self.sync();
        let Some(shown) = self.queue.first() else {
            return;
        };
        let notification = &shown.notification;
        let theme = ui::theme(ui.ctx());

        ui::panel(&theme).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&notification.summary).strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(
                            RichText::new(&notification.app_name)
                                .color(theme.subtext)
                                .small(),
                        );
                    });
                });
                if !notification.body.is_empty() {
                    ui.label(&notification.body);
                }
            });
        });

        if ui.input(|i| i.pointer.any_click()) {
            self.queue.remove(0);
            ui.ctx().request_repaint();
        } else if let Some(expires) = shown.expires_at() {
            ui.ctx()
                .request_repaint_after(expires.saturating_duration_since(Instant::now()));
        }
    }

    fn visible(&mut self) -> bool {
        self.sync();
        !self.queue.is_empty()
    }
}
