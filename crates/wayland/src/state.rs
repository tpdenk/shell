//! Dispatch state: implements the smithay-client-toolkit handler traits for
//! the globals the shell binds, and drives egui frames on the layer surface.

use std::num::NonZeroU32;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{LoopHandle, LoopSignal, RegistrationToken};
use smithay_client_toolkit::reexports::client::globals::GlobalList;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_output, wl_pointer, wl_seat, wl_surface,
};
use smithay_client_toolkit::reexports::client::{Connection, QueueHandle};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{
    PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};

use crate::App;
use crate::gpu::Gpu;
use crate::input::{InputState, cursor_icon};

pub(crate) struct State {
    conn: Connection,
    qh: QueueHandle<State>,
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    compositor: CompositorState,
    shm: Shm,
    layer: LayerSurface,
    gpu: Gpu,
    app: Box<dyn App>,
    egui: egui::Context,
    input: InputState,
    pointer: Option<ThemedPointer>,
    cursor: egui::CursorIcon,
    loop_handle: LoopHandle<'static, State>,
    loop_signal: LoopSignal,
    started: Instant,
    repaint_timer: Option<RegistrationToken>,

    /// Surface size in logical pixels, as last configured by the compositor.
    size: (u32, u32),
    /// Integer buffer scale. Buffers are `size * scale` pixels.
    scale: i32,
    /// The surface has a buffer attached and is shown by the compositor.
    mapped: bool,
    /// An initial commit was sent and its configure is still outstanding.
    map_requested: bool,
    /// A configure was received since the last unmap, so a buffer may be
    /// attached at any time.
    configured: bool,
    /// A frame callback is outstanding. Prevents queueing more than one.
    frame_pending: bool,
    /// Something changed since the last draw.
    dirty: bool,
}

impl State {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        conn: Connection,
        globals: &GlobalList,
        qh: &QueueHandle<State>,
        compositor: CompositorState,
        shm: Shm,
        layer: LayerSurface,
        gpu: Gpu,
        egui: egui::Context,
        app: Box<dyn App>,
        loop_handle: LoopHandle<'static, State>,
        loop_signal: LoopSignal,
    ) -> State {
        State {
            conn,
            qh: qh.clone(),
            registry_state: RegistryState::new(globals),
            output_state: OutputState::new(globals, qh),
            seat_state: SeatState::new(globals, qh),
            compositor,
            shm,
            layer,
            gpu,
            app,
            egui,
            input: InputState::default(),
            pointer: None,
            cursor: egui::CursorIcon::Default,
            loop_handle,
            loop_signal,
            started: Instant::now(),
            repaint_timer: None,
            size: (0, 0),
            scale: 1,
            mapped: false,
            map_requested: false,
            configured: false,
            frame_pending: false,
            dirty: true,
        }
    }

    /// Schedules a redraw for the compositor's next frame. Cheap to call
    /// repeatedly. While unmapped this instead drives the map sequence if the
    /// app wants to be shown.
    pub(crate) fn request_redraw(&mut self) {
        self.dirty = true;
        if !self.mapped {
            if self.configured {
                // The compositor already told us our size. Attaching a buffer
                // maps the surface, no frame callback needed.
                self.draw();
            } else {
                self.request_map();
            }
            return;
        }
        if self.frame_pending {
            return;
        }
        self.frame_pending = true;
        let surface = self.layer.wl_surface();
        surface.frame(&self.qh, FrameCallbackData(surface.clone()));
        surface.commit();
    }

    /// Initial commit without a buffer. The compositor answers with a
    /// configure, and `draw` runs from there.
    fn request_map(&mut self) {
        if self.map_requested || !self.app.visible() {
            return;
        }
        self.map_requested = true;
        self.layer.commit();
    }

    /// Attaching no buffer unmaps a layer surface. It returns to its initial
    /// state, so showing it again goes through `request_map`.
    fn unmap(&mut self) {
        let surface = self.layer.wl_surface();
        surface.attach(None, 0, 0);
        surface.commit();
        self.mapped = false;
        self.map_requested = false;
        self.configured = false;
        self.frame_pending = false;
        self.dirty = false;
        self.input.pointer_inside = false;
        if let Some(token) = self.repaint_timer.take() {
            self.loop_handle.remove(token);
        }
    }

    fn schedule_repaint(&mut self, delay: Duration) {
        if let Some(token) = self.repaint_timer.take() {
            self.loop_handle.remove(token);
        }
        if delay.is_zero() {
            self.request_redraw();
            return;
        }
        // Anything beyond a few seconds is egui's "never" placeholder.
        if delay > Duration::from_secs(60) {
            return;
        }
        let token = self
            .loop_handle
            .insert_source(Timer::from_duration(delay), |_, _, state: &mut State| {
                state.repaint_timer = None;
                state.request_redraw();
                TimeoutAction::Drop
            })
            .expect("inserting repaint timer");
        self.repaint_timer = Some(token);
    }

    fn resize_gpu(&mut self) {
        let (w, h) = self.size;
        let scale = self.scale as u32;
        self.gpu.resize(w * scale, h * scale);
    }

    fn draw(&mut self) {
        if !self.app.visible() {
            if self.mapped {
                self.unmap();
            } else {
                self.map_requested = false;
            }
            return;
        }
        let (width, height) = self.size;
        if width == 0 || height == 0 || !self.gpu.is_configured() {
            return;
        }
        let pixels_per_point = self.scale as f32;

        let mut raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width as f32, height as f32),
            )),
            time: Some(self.started.elapsed().as_secs_f64()),
            events: self.input.take(),
            focused: true,
            ..Default::default()
        };
        raw_input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .native_pixels_per_point = Some(pixels_per_point);

        // Request the next frame callback before presenting. The commit that
        // Vulkan's WSI performs on present carries this request along.
        let surface = self.layer.wl_surface();
        surface.frame(&self.qh, FrameCallbackData(surface.clone()));
        self.frame_pending = true;

        let app = &mut self.app;
        let output = self.egui.run_ui(raw_input, |ui| app.ui(ui));
        let paint_jobs = self.egui.tessellate(output.shapes, output.pixels_per_point);
        let presented =
            self.gpu
                .render(output.pixels_per_point, output.textures_delta, &paint_jobs);
        self.set_cursor(output.platform_output.cursor_icon);

        if self.app.wants_exit() {
            self.loop_signal.stop();
            return;
        }

        if presented {
            self.mapped = true;
            self.map_requested = false;
            self.dirty = false;
            if let Some(root) = output.viewport_output.get(&egui::ViewportId::ROOT) {
                self.schedule_repaint(root.repaint_delay);
            }
        } else if self.mapped {
            // Flush the frame request so the callback still fires and the
            // frame is retried.
            self.layer.commit();
            self.dirty = true;
        } else {
            // Nothing is mapped, so no frame callback will come. The pending
            // configure stays valid, retry shortly.
            self.frame_pending = false;
            self.dirty = true;
            self.schedule_repaint(Duration::from_millis(16));
        }
    }

    fn set_cursor(&mut self, icon: egui::CursorIcon) {
        if icon == self.cursor && self.pointer.is_some() {
            return;
        }
        self.cursor = icon;
        self.apply_cursor();
    }

    /// Pushes the current cursor to the compositor. Needed after every pointer
    /// enter as well as when egui changes it.
    fn apply_cursor(&mut self) {
        let Some(pointer) = &self.pointer else { return };
        if !self.input.pointer_inside {
            return;
        }
        let result = match cursor_icon(self.cursor) {
            Some(icon) => pointer.set_cursor(&self.conn, icon),
            None => pointer.hide_cursor(),
        };
        if let Err(err) = result {
            log::debug!("failed to set cursor {:?}: {err}", self.cursor);
        }
    }
}

impl LayerShellHandler for State {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        log::info!("layer surface closed by the compositor");
        self.loop_signal.stop();
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let (w, h) = configure.new_size;
        // Zero means "you choose". Keep whatever was configured before.
        self.size = (
            NonZeroU32::new(w).map_or(self.size.0, NonZeroU32::get),
            NonZeroU32::new(h).map_or(self.size.1, NonZeroU32::get),
        );
        log::debug!(
            "configured layer surface to {}x{}",
            self.size.0,
            self.size.1
        );
        self.resize_gpu();
        self.configured = true;
        // Every configure must be answered with a commit. Presenting does that.
        self.draw();
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        surface: &wl_surface::WlSurface,
        new_factor: i32,
    ) {
        if self.scale == new_factor {
            return;
        }
        self.scale = new_factor;
        surface.set_buffer_scale(new_factor);
        self.resize_gpu();
        self.request_redraw();
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        self.frame_pending = false;
        if self.dirty {
            self.draw();
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability != Capability::Pointer || self.pointer.is_some() {
            return;
        }
        let cursor_surface = self.compositor.create_surface(qh);
        match self.seat_state.get_pointer_with_theme::<State, ()>(
            qh,
            &seat,
            self.shm.wl_shm(),
            cursor_surface,
            ThemeSpec::default(),
        ) {
            Ok(pointer) => self.pointer = Some(pointer),
            Err(err) => log::error!("failed to create pointer: {err}"),
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            // Dropping the themed pointer releases the wl_pointer.
            self.pointer = None;
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let mut redraw = false;
        for event in events {
            if &event.surface != self.layer.wl_surface() {
                continue;
            }
            redraw |= self.input.push_pointer(event);
            if matches!(event.kind, PointerEventKind::Enter { .. }) {
                self.apply_cursor();
            }
        }
        if redraw {
            self.request_redraw();
        }
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}

delegate_registry!(State);
smithay_client_toolkit::delegate_dispatch2!(State);
