//! The window of the radial menu: a layer-shell surface over the whole output
//! (KDE, wlroots compositors), shown when the menu opens and destroyed when it
//! closes. It draws what the [`Controller`](super::controller::Controller)
//! says and reports the pointer, the clicks and Escape back.
//!
//! Wayland gives a client no pointer position outside its windows, so the
//! surface covers the output: the pointer's first position over it is where
//! the menu opens (`WindowEvent::Pointer`).

use std::sync::Arc;
use std::time::{Duration, Instant};

use smithay_client_toolkit::compositor::{CompositorHandler, CompositorState, FrameCallbackData};
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop::channel::{self, Channel, Event as ChannelEvent};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::reexports::calloop::{EventLoop, LoopSignal};
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::reexports::client::globals::registry_queue_init;
use smithay_client_toolkit::reexports::client::protocol::{
    wl_keyboard, wl_output, wl_pointer, wl_seat, wl_shm, wl_surface,
};
use smithay_client_toolkit::reexports::client::{Connection, Dispatch, QueueHandle, WEnum};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::seat::pointer::{
    CursorIcon, PointerEvent, PointerEventKind, PointerHandler, ThemeSpec, ThemedPointer,
};
use smithay_client_toolkit::seat::{Capability, SeatHandler, SeatState};
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
    LayerSurfaceConfigure,
};
use smithay_client_toolkit::shm::slot::SlotPool;
use smithay_client_toolkit::shm::{Shm, ShmHandler};
use smithay_client_toolkit::{delegate_registry, registry_handlers};
use thiserror::Error;

use super::controller::{View, WindowEvent};
use super::geometry::{Point, Size};
use super::paint::{PetalView, paint};
use super::window::{
    BYTES_PER_PIXEL, Content, Gone, MAX_SURFACE_SIDE, Report, Surface, blit, has_expired,
    opening_progress, pixels, signed,
};
use crate::render::{RenderError, TextRenderer};

/// Linux key code of Escape (`linux/input-event-codes.h`).
const KEY_ESCAPE: u32 = 1;

/// How often the window looks at how long it has been open.
const WATCHDOG_PERIOD: Duration = Duration::from_secs(1);

/// Namespace of the surface, for the compositor's rules.
const NAMESPACE: &str = "hubergamof-radial-menu";

/// What the window is asked to do.
#[derive(Debug)]
enum Command {
    Open(Box<Content>),
    Place(View),
    Redraw(View),
    Close,
    Stop,
}

/// Why the window cannot be started.
#[derive(Debug, Error)]
pub enum OverlayError {
    /// No Wayland session, or the compositor refuses the connection.
    #[error("cannot connect to the Wayland compositor: {0}")]
    Connect(String),
    /// The compositor lacks a protocol the window needs (GNOME has no
    /// layer-shell).
    #[error("the compositor does not offer {0}")]
    MissingProtocol(&'static str),
    /// The font of the labels cannot be loaded.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// The thread of the window cannot be started.
    #[error("cannot start the window thread: {0}")]
    Thread(String),
}

/// The window of the menu, running on its own thread.
#[derive(Debug)]
pub struct Overlay {
    commands: channel::Sender<Command>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Overlay {
    /// Connects to the compositor and starts the thread of the window, which
    /// reports the pointer, the clicks and Escape to `events`.
    ///
    /// # Errors
    ///
    /// [`OverlayError`] when there is no compositor, it has no layer-shell, or
    /// the thread cannot start.
    pub fn start(events: Report) -> Result<Self, OverlayError> {
        let connection = Connection::connect_to_env()
            .map_err(|error| OverlayError::Connect(error.to_string()))?;
        let (globals, queue) = registry_queue_init::<State>(&connection)
            .map_err(|error| OverlayError::Connect(error.to_string()))?;
        let handle = queue.handle();
        let compositor = CompositorState::bind(&globals, &handle)
            .map_err(|_| OverlayError::MissingProtocol("wl_compositor"))?;
        let layer_shell = LayerShell::bind(&globals, &handle)
            .map_err(|_| OverlayError::MissingProtocol("zwlr_layer_shell_v1"))?;
        let shm =
            Shm::bind(&globals, &handle).map_err(|_| OverlayError::MissingProtocol("wl_shm"))?;
        let pool = SlotPool::new(BYTES_PER_PIXEL, &shm)
            .map_err(|error| OverlayError::Connect(error.to_string()))?;
        let (commands, receiver) = channel::channel();
        let mut state = State {
            registry: RegistryState::new(&globals),
            seats: SeatState::new(&globals, &handle),
            outputs: OutputState::new(&globals, &handle),
            compositor,
            layer_shell,
            shm,
            pool,
            renderer: TextRenderer::new()?,
            queue: handle,
            events,
            pointer: None,
            keyboard: None,
            window: None,
            signal: None,
        };
        let report = Arc::clone(&state.events);
        let thread = std::thread::Builder::new()
            .name("radial-overlay".to_owned())
            .spawn(move || {
                // Told when the thread ends, whatever the way (an error, a
                // panic unwinding): the host must not go on with a menu that
                // has no window.
                let _gone = Gone(report);
                if let Err(error) = run(connection, queue, receiver, &mut state) {
                    tracing::error!(%error, "the radial menu window stopped");
                }
            })
            .map_err(|error| OverlayError::Thread(error.to_string()))?;
        Ok(Self {
            commands,
            thread: Some(thread),
        })
    }

    /// Shows the window; the menu is drawn once it is placed.
    pub fn open(&self, content: Content) {
        self.send(Command::Open(Box::new(content)));
    }

    /// Draws the menu at its place.
    pub fn place(&self, view: View) {
        self.send(Command::Place(view));
    }

    /// Draws the menu again, a petal being lit or not.
    pub fn redraw(&self, view: View) {
        self.send(Command::Redraw(view));
    }

    /// Hides the window.
    pub fn close(&self) {
        self.send(Command::Close);
    }

    fn send(&self, command: Command) {
        // The thread is gone when it failed: it has said so, and the menu
        // simply does not show.
        let _ = self.commands.send(command);
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        self.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Runs the loop of the window until it is told to stop.
fn run(
    connection: Connection,
    queue: smithay_client_toolkit::reexports::client::EventQueue<State>,
    commands: Channel<Command>,
    state: &mut State,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let handle = event_loop.handle();
    WaylandSource::new(connection, queue).insert(handle.clone())?;
    handle
        .insert_source(commands, |event, (), state| match event {
            ChannelEvent::Msg(command) => state.command(command),
            ChannelEvent::Closed => state.stop(),
        })
        .map_err(|error| error.error)?;
    handle
        .insert_source(Timer::from_duration(WATCHDOG_PERIOD), |_, (), state| {
            state.close_if_forgotten();
            TimeoutAction::ToDuration(WATCHDOG_PERIOD)
        })
        .map_err(|error| error.error)?;
    state.signal = Some(event_loop.get_signal());
    event_loop.run(None, state, |_| {})?;
    Ok(())
}

/// The window being shown.
struct Window {
    layer: LayerSurface,
    /// When the window was shown, for [`MAX_OPEN`].
    opened_at: Instant,
    content: Content,
    /// Size of the surface, known at the first configure.
    size: Option<Size>,
    /// Where and what to draw, once the menu is placed.
    view: Option<View>,
    /// When the menu was placed, to animate the opening.
    placed_at: Option<Instant>,
    /// A frame callback is pending: no other drawing is queued meanwhile.
    frame_requested: bool,
}

struct State {
    registry: RegistryState,
    seats: SeatState,
    outputs: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    renderer: TextRenderer,
    queue: QueueHandle<State>,
    events: Report,
    pointer: Option<ThemedPointer>,
    keyboard: Option<wl_keyboard::WlKeyboard>,
    window: Option<Window>,
    signal: Option<LoopSignal>,
}

impl State {
    fn command(&mut self, command: Command) {
        match command {
            Command::Open(content) => self.show(*content),
            Command::Place(view) => {
                if let Some(window) = &mut self.window {
                    window.view = Some(view);
                    window.placed_at = Some(Instant::now());
                }
                self.draw();
            }
            Command::Redraw(view) => {
                if let Some(window) = &mut self.window {
                    window.view = Some(view);
                }
                self.draw();
            }
            Command::Close => self.window = None,
            Command::Stop => self.stop(),
        }
    }

    /// Closes the window, whatever the host does, when it has been open too
    /// long; the host is told as if the person gave up.
    fn close_if_forgotten(&mut self) {
        let forgotten = self
            .window
            .as_ref()
            .is_some_and(|window| has_expired(window.opened_at, Instant::now()));
        if forgotten {
            tracing::warn!("the radial menu stayed open too long: closed");
            self.give_up();
        }
    }

    /// The window goes and the host hears Escape: Escape, a window the
    /// compositor took away, a host that is no longer there.
    fn give_up(&mut self) {
        self.window = None;
        // The host may be gone: nothing to tell then, the window is closed anyway.
        let _ = (self.events)(WindowEvent::Escape);
    }

    fn stop(&mut self) {
        self.window = None;
        if let Some(signal) = &self.signal {
            signal.stop();
        }
    }

    fn show(&mut self, content: Content) {
        let handle = self.queue.clone();
        let surface = self.compositor.create_surface(&handle);
        let layer = self.layer_shell.create_layer_surface(
            &handle,
            surface,
            Layer::Overlay,
            Some(NAMESPACE),
            None,
        );
        layer.set_anchor(Anchor::all());
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_size(0, 0);
        // The first commit, without a buffer, asks for the configure that
        // gives the size of the output.
        layer.commit();
        self.window = Some(Window {
            layer,
            opened_at: Instant::now(),
            content,
            size: None,
            view: None,
            placed_at: None,
            frame_requested: false,
        });
    }
}

impl State {
    /// Draws the menu (or, before it is placed, nothing) into a new buffer.
    fn draw(&mut self) {
        let Some(window) = &mut self.window else {
            return;
        };
        let Some(size) = window.size else {
            return;
        };
        let (width, height) = (pixels(size.width), pixels(size.height));
        // Sizes are capped at the configure: the stride cannot overflow.
        let stride = signed(width).saturating_mul(4);
        let Ok((buffer, canvas)) = self.pool.create_buffer(
            signed(width),
            signed(height),
            stride,
            wl_shm::Format::Argb8888,
        ) else {
            tracing::warn!("cannot allocate a buffer for the radial menu");
            return;
        };
        canvas.fill(0);
        let mut animating = false;
        if let Some(view) = window.view {
            let opening = opening_progress(&window.content, window.placed_at);
            animating = opening < 1.0;
            let petals: Vec<PetalView<'_>> = window
                .content
                .petals
                .iter()
                .map(|petal| PetalView {
                    label: &petal.label,
                    icon: petal.icon.as_ref(),
                })
                .collect();
            match paint(
                &self.renderer,
                &window.content.layout,
                &petals,
                view.lit,
                opening,
                window.content.palette,
            ) {
                Ok(image) => blit(canvas, width, height, &image, view.centre),
                Err(error) => tracing::warn!(%error, "cannot draw the radial menu"),
            }
        }
        let surface = window.layer.wl_surface();
        surface.damage_buffer(0, 0, signed(width), signed(height));
        if animating && !window.frame_requested {
            surface.frame(&self.queue, FrameCallbackData(surface.clone()));
            window.frame_requested = true;
        }
        if buffer.attach_to(surface).is_ok() {
            window.layer.commit();
        }
    }

    /// Puts the plain arrow over the menu, whatever the pointer showed before.
    fn show_default_cursor(&self, connection: &Connection) {
        if let Some(pointer) = &self.pointer
            && let Err(error) = pointer.set_cursor(connection, CursorIcon::Default)
        {
            tracing::debug!(%error, "cannot set the cursor over the radial menu");
        }
    }

    /// Tells the host; a host that is gone cannot close the window, so the
    /// window closes itself.
    fn report(&mut self, event: WindowEvent) {
        if !(self.events)(event) {
            self.window = None;
        }
    }
}

impl CompositorHandler for State {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: wl_output::Transform,
    ) {
    }

    fn frame(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &wl_surface::WlSurface, _: u32) {
        if let Some(window) = &mut self.window {
            window.frame_requested = false;
        }
        self.draw();
    }

    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_surface::WlSurface,
        _: &wl_output::WlOutput,
    ) {
    }
}

impl OutputHandler for State {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.outputs
    }

    fn new_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}

    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_output::WlOutput) {}
}

impl LayerShellHandler for State {
    fn closed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &LayerSurface) {
        // The compositor took the surface away (the output went): the menu is
        // given up, as with Escape.
        self.give_up();
    }

    fn configure(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _: u32,
    ) {
        let (width, height) = configure.new_size;
        if let Some(window) = &mut self.window
            && (1..=MAX_SURFACE_SIDE).contains(&width)
            && (1..=MAX_SURFACE_SIDE).contains(&height)
        {
            let size = Size {
                width: f64::from(width),
                height: f64::from(height),
            };
            window.size = Some(size);
            self.report(WindowEvent::Screen(size));
        }
        // The first buffer maps the surface, and the pointer enters it.
        self.draw();
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seats
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}

    fn new_capability(
        &mut self,
        _: &Connection,
        queue: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer if self.pointer.is_none() => {
                // The cursor belongs to the client that has the pointer: without
                // one set, the pointer keeps the shape it had over the window
                // below (a text caret, or nothing at all).
                let cursor_surface = self.compositor.create_surface(queue);
                self.pointer = self
                    .seats
                    .get_pointer_with_theme::<State, ()>(
                        queue,
                        &seat,
                        self.shm.wl_shm(),
                        cursor_surface,
                        ThemeSpec::System,
                    )
                    .ok();
            }
            Capability::Keyboard if self.keyboard.is_none() => {
                self.keyboard = Some(seat.get_keyboard(queue, ()));
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: wl_seat::WlSeat,
        capability: Capability,
    ) {
        match capability {
            Capability::Pointer => {
                if let Some(pointer) = self.pointer.take() {
                    pointer.pointer().release();
                }
            }
            Capability::Keyboard => {
                if let Some(keyboard) = self.keyboard.take() {
                    keyboard.release();
                }
            }
            _ => {}
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: wl_seat::WlSeat) {}
}

impl PointerHandler for State {
    fn pointer_frame(
        &mut self,
        connection: &Connection,
        _: &QueueHandle<Self>,
        _: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        let Some(window) = &self.window else {
            return;
        };
        let ours = window.layer.wl_surface().clone();
        // A fast mouse sends many motions at once: only where it ended up
        // matters, and a click comes after.
        let mut last_position = None;
        let mut clicked = false;
        for event in events.iter().filter(|event| event.surface == ours) {
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    // The shape is to be set at every enter.
                    self.show_default_cursor(connection);
                    last_position = Some(Point::new(event.position.0, event.position.1));
                }
                PointerEventKind::Motion { .. } => {
                    last_position = Some(Point::new(event.position.0, event.position.1));
                }
                PointerEventKind::Press { .. } => clicked = true,
                PointerEventKind::Leave { .. }
                | PointerEventKind::Release { .. }
                | PointerEventKind::Axis { .. } => {}
            }
        }
        if let Some(position) = last_position {
            self.report(WindowEvent::Pointer(position));
        }
        if clicked {
            self.report(WindowEvent::Click);
        }
    }
}

/// Only Escape is read from the keyboard, by its key code: the key maps (and
/// the system library that reads them) are not needed for that.
impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        (): &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Key {
            key,
            state: WEnum::Value(wl_keyboard::KeyState::Pressed),
            ..
        } = event
            && key == KEY_ESCAPE
        {
            state.give_up();
        }
    }
}

impl ShmHandler for State {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

delegate_registry!(State);

impl ProvidesRegistryState for State {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
    registry_handlers![OutputState, SeatState];
}

smithay_client_toolkit::delegate_dispatch2!(State);

impl Surface for Overlay {
    fn open(&self, content: Content) {
        Self::open(self, content);
    }

    fn place(&self, view: View) {
        Self::place(self, view);
    }

    fn redraw(&self, view: View) {
        Self::redraw(self, view);
    }

    fn close(&self) {
        Self::close(self);
    }
}
