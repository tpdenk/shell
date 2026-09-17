//! Wayland platform layer.
//!
//! Connects to the compositor, maps one `wlr-layer-shell` surface, and renders
//! an egui [`App`] onto it with wgpu. The process is driven by a `calloop`
//! event loop so timers and IPC sockets can be added as further sources.

mod gpu;
mod input;
mod state;

use anyhow::{Context, Result};
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop::ping::make_ping;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::Connection;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::LayerShell;
pub use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use smithay_client_toolkit::shm::Shm;

use crate::gpu::Gpu;
use crate::state::State;

/// The UI hosted on a layer surface.
pub trait App {
    /// Draws one frame. `ui` covers the whole surface, with no margin or
    /// background. Use `ui.ctx().request_repaint()` (from any thread, via a
    /// cloned [`egui::Context`]) to schedule another frame.
    fn ui(&mut self, ui: &mut egui::Ui);

    /// Whether the surface should be shown at all. While `false` the surface
    /// is unmapped: it takes no screen space and receives no input. Called
    /// before every frame and whenever a repaint is requested.
    fn visible(&mut self) -> bool {
        true
    }

    /// Returning `true` ends [`run`] after the current frame.
    fn wants_exit(&self) -> bool {
        false
    }
}

/// Margins around the surface, in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Margin {
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub left: i32,
}

impl Margin {
    pub const fn same(px: i32) -> Margin {
        Margin {
            top: px,
            right: px,
            bottom: px,
            left: px,
        }
    }
}

#[derive(Clone, Debug)]
pub struct LayerConfig {
    /// Layer-shell namespace, visible to the compositor (e.g. for layer rules).
    pub namespace: String,
    pub layer: Layer,
    /// Edges the surface sticks to. Anchoring opposite edges stretches the
    /// surface along that axis, in which case that component of `size` must
    /// be zero. `Anchor::empty()` centers the surface.
    pub anchor: Anchor,
    /// Requested size in logical pixels. Zero means "stretch", which is only
    /// valid along an axis anchored on both sides.
    pub size: (u32, u32),
    pub margin: Margin,
    /// Screen area to reserve so tiled windows do not overlap the surface.
    /// Zero reserves nothing, `-1` ignores other surfaces' exclusive zones.
    pub exclusive_zone: i32,
    pub keyboard: KeyboardInteractivity,
}

/// Maps the layer surface and runs the event loop until the compositor closes
/// the surface, the app asks to exit, or the connection fails.
///
/// `make_app` receives the [`egui::Context`]. Clones of it are `Send` and can
/// be handed to other threads to request repaints.
pub fn run(
    config: LayerConfig,
    make_app: impl FnOnce(&egui::Context) -> Box<dyn App>,
) -> Result<()> {
    let conn = Connection::connect_to_env().context("connecting to the Wayland display")?;
    let (globals, event_queue) =
        registry_queue_init::<State>(&conn).context("initializing the Wayland registry")?;
    let qh = event_queue.handle();

    let compositor =
        CompositorState::bind(&globals, &qh).context("wl_compositor is unavailable")?;
    let layer_shell =
        LayerShell::bind(&globals, &qh).context("zwlr_layer_shell_v1 is unavailable")?;
    // Only used for cursor themes when the compositor lacks cursor-shape-v1.
    let shm = Shm::bind(&globals, &qh).context("wl_shm is unavailable")?;

    let surface = compositor.create_surface(&qh);
    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        config.layer,
        Some(config.namespace.as_str()),
        None,
    );
    layer.set_anchor(config.anchor);
    layer.set_size(config.size.0, config.size.1);
    let m = config.margin;
    layer.set_margin(m.top, m.right, m.bottom, m.left);
    layer.set_exclusive_zone(config.exclusive_zone);
    layer.set_keyboard_interactivity(config.keyboard);

    let gpu = Gpu::new(&conn, layer.wl_surface()).context("initializing the GPU")?;

    let mut event_loop: EventLoop<'static, State> =
        EventLoop::try_new().context("creating the event loop")?;
    WaylandSource::new(conn.clone(), event_queue)
        .insert(event_loop.handle())
        .map_err(|e| e.error)
        .context("registering the Wayland event source")?;

    // Any thread can wake the loop through `egui::Context::request_repaint`.
    let (ping, ping_source) = make_ping().context("creating the repaint wake source")?;
    event_loop
        .handle()
        .insert_source(ping_source, |_, _, state: &mut State| {
            state.request_redraw()
        })
        .map_err(|e| e.error)
        .context("registering the repaint wake source")?;
    let egui = egui::Context::default();
    egui.set_request_repaint_callback(move |_| ping.ping());

    let app = make_app(&egui);
    let mut state = State::new(
        conn,
        &globals,
        &qh,
        compositor,
        shm,
        layer,
        gpu,
        egui,
        app,
        event_loop.handle(),
        event_loop.get_signal(),
    );
    // Performs the initial commit if the app starts visible. The compositor
    // answers with a configure carrying the real size, and the first frame
    // is drawn from there.
    state.request_redraw();

    event_loop
        .run(None, &mut state, |_| {})
        .context("running the event loop")?;
    Ok(())
}
