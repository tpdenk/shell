use std::sync::Arc;
use std::thread;
use std::time::Duration;

use egui::{
    Button, Color32, Context, CornerRadius, Id, Response, RichText, Stroke, Ui, Widget, vec2,
};
use hypr::{Instance, WorkspaceId, WorkspaceSelector};
use log::error;

use crate::WorkspacesConfig;

const SNAPSHOT_ID: &str = "bar.workspaces";
const SHARED_ID: &str = "bar.workspaces.shared";

/// Names of the events after which the snapshot may be stale.
const REFRESH_EVENTS: &[&str] = &[
    "workspace",
    "createworkspace",
    "destroyworkspace",
    "moveworkspace",
    "renameworkspace",
    "focusedmon",
    "monitoradded",
    "monitorremoved",
    "openwindow",
    "closewindow",
    "movewindow",
];

/// Set once by [`install`], read by the widget on every frame and click.
struct Shared {
    config: WorkspacesConfig,
    instance: Instance,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Snapshot {
    workspaces: Vec<Entry>,
    active: Option<WorkspaceId>,
    visible: Vec<WorkspaceId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    id: WorkspaceId,
    name: String,
    windows: u16,
}

pub fn install(ctx: &Context, config: WorkspacesConfig) {
    let instance = match Instance::from_env() {
        Ok(instance) => instance,
        Err(err) => {
            log::warn!("workspaces unavailable: {err}");
            return;
        }
    };
    let shared = Arc::new(Shared { config, instance });
    ctx.data_mut(|d| d.insert_temp(Id::new(SHARED_ID), shared.clone()));
    let spawned = thread::Builder::new().name("workspaces".into()).spawn({
        let ctx = ctx.clone();
        move || watch(ctx, &shared)
    });
    if let Err(err) = spawned {
        log::warn!("workspaces will not update: {err}");
    }
}

fn watch(ctx: Context, shared: &Shared) {
    let refresh = || match fetch(shared) {
        Ok(next) => {
            let changed = ctx.data_mut(|d| {
                let id = Id::new(SNAPSHOT_ID);
                let current = d.get_temp::<Arc<Snapshot>>(id);
                if current.as_deref() == Some(&next) {
                    false
                } else {
                    d.insert_temp(id, Arc::new(next));
                    true
                }
            });
            if changed {
                ctx.request_repaint();
            }
        }
        Err(err) => log::warn!("reading Hyprland workspaces: {err:#}"),
    };
    loop {
        refresh();
        let events = match shared.instance.events() {
            Ok(events) => events,
            Err(err) => {
                log::warn!("connecting to Hyprland events, retrying: {err:#}");
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        for event in events {
            match event {
                Ok(event) if REFRESH_EVENTS.contains(&event.name.as_str()) => refresh(),
                Ok(_) => {}
                Err(err) => {
                    log::warn!("Hyprland event stream ended, reconnecting: {err}");
                    thread::sleep(Duration::from_secs(1));
                    break;
                }
            }
        }
    }
}

fn fetch(shared: &Shared) -> anyhow::Result<Snapshot> {
    let config = &shared.config;
    let monitors = shared.instance.monitors()?;
    let mut workspaces: Vec<Entry> = shared
        .instance
        .workspaces()?
        .into_iter()
        .filter(|w| (config.show_special || w.id > 0) && (config.show_empty || w.windows > 0))
        .map(|w| Entry {
            id: w.id,
            name: w.name,
            windows: w.windows,
        })
        .collect();
    workspaces.sort_by_key(|w| w.id);
    let active = monitors
        .iter()
        .find(|m| m.focused)
        .map(|m| m.active_workspace.id);
    let visible = monitors.iter().map(|m| m.active_workspace.id).collect();
    Ok(Snapshot {
        workspaces,
        active,
        visible,
    })
}

pub struct WorkspacesWidget;

impl Widget for WorkspacesWidget {
    fn ui(self, ui: &mut Ui) -> Response {
        let theme = ui::theme(ui.ctx());
        let (shared, snapshot) = ui.data(|d| {
            (
                d.get_temp::<Arc<Shared>>(Id::new(SHARED_ID)),
                d.get_temp::<Arc<Snapshot>>(Id::new(SNAPSHOT_ID)),
            )
        });

        // Without Hyprland there is nothing to show.
        let Some(shared) = shared else {
            return ui.response();
        };

        let config = &shared.config;
        let snapshot = snapshot.unwrap_or_default();
        let height = ui.available_height();
        let radius = CornerRadius::same(theme.rounding.min(config.max_rounding));
        ui.spacing_mut().item_spacing.x = config.spacing;
        ui.scope(|ui| {
            for entry in &snapshot.workspaces {
                let is_active = snapshot.active == Some(entry.id);
                let is_visible = snapshot.visible.contains(&entry.id);
                let (fill, stroke, text) = if is_active {
                    (theme.accent, Stroke::NONE, theme.background.to_opaque())
                } else if is_visible {
                    (
                        theme.surface_hi,
                        Stroke::new(config.visible_stroke, theme.accent),
                        theme.text,
                    )
                } else if entry.windows > 0 {
                    (theme.surface, Stroke::NONE, theme.text)
                } else {
                    (Color32::TRANSPARENT, Stroke::NONE, theme.subtext)
                };
                let button = Button::new(RichText::new(&entry.name).color(text))
                    .fill(fill)
                    .stroke(stroke)
                    .corner_radius(radius)
                    .min_size(vec2(height, height));
                if ui.add(button).clicked() && !is_active {
                    // Special workspaces have negative ids. Their name
                    // (`special:foo`) is the selector Hyprland resolves.
                    let target = if entry.id > 0 {
                        WorkspaceSelector::Id(entry.id)
                    } else {
                        WorkspaceSelector::Name(&entry.name)
                    };
                    if let Err(err) = shared.instance.focus_workspace(target) {
                        error!("switching to workspace {}: {err:#}", entry.name);
                    }
                }
            }
        })
        .response
    }
}
