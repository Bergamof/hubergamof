//! The window of the radial menu on X11: for the desktops that have no
//! layer-shell (GNOME), through Xorg or `XWayland`. A borderless full-screen
//! window with an alpha channel, so the menu is drawn over what is below; it is
//! mapped when the menu opens and unmapped when it closes.
//!
//! Like the layer-shell window, it covers the screen so that the pointer's first
//! position over it is where the menu opens, and it closes itself after
//! [`MAX_OPEN`] whatever the host does.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, Instant};

use thiserror::Error;
use x11rb::CURRENT_TIME;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ColormapAlloc, ConnectionExt as _, CreateGCAux,
    CreateWindowAux, EventMask, GrabMode, GrabStatus, ImageFormat, PropMode, VisualClass, Visualid,
    Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use super::controller::{View, WindowEvent};
use super::geometry::{Point, Size};
use super::paint::{PetalView, paint};
use super::window::{
    Content, Gone, MAX_OPEN, MAX_SURFACE_SIDE, Report, Surface, has_expired, opening_progress,
    premultiplied_bgra,
};
use crate::render::{RenderError, TextRenderer};

/// X11 key code of Escape (the Linux key code, 1, plus the offset X11 adds).
const KEYCODE_ESCAPE: u8 = 9;

/// Glyph of the plain arrow in the `cursor` font (`X11/cursorfont.h`,
/// `XC_left_ptr`): without a cursor of its own the window shows the one of
/// its parent, the root.
const CURSOR_GLYPH_ARROW: u16 = 68;

/// Colour of a cursor, 16 bits a channel: black arrow, white mask.
const CURSOR_FOREGROUND: (u16, u16, u16) = (0, 0, 0);
const CURSOR_BACKGROUND: (u16, u16, u16) = (u16::MAX, u16::MAX, u16::MAX);

/// Depth of the visuals with an alpha channel.
const ARGB_DEPTH: u8 = 32;

/// Colour masks of the 32-bit visual that is `0xAARRGGBB`.
const RED_MASK: u32 = 0x00ff_0000;
const GREEN_MASK: u32 = 0x0000_ff00;
const BLUE_MASK: u32 = 0x0000_00ff;

/// How long the menu is drawn again and again after it is placed: a desktop
/// can show the window a moment after the first drawings, which are then lost.
const SETTLE: Duration = Duration::from_secs(1);

/// Time between two frames while the menu opens.
const FRAME_PERIOD: Duration = Duration::from_millis(16);

/// Name of the atom of the message that wakes the thread.
const WAKE_ATOM_NAME: &[u8] = b"_HUBERGAMOF_RADIAL_WAKE";

/// Most X events taken before the commands of the host are looked at: a client
/// that floods the window cannot keep `Stop` from being read.
const MAX_BATCH: usize = 256;

/// Name of the window, and class for the window manager's rules.
const WINDOW_NAME: &str = "HuBergamof radial menu";
const WINDOW_CLASS: &[u8] = b"hubergamof-radial-menu\0HuBergamof\0";

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
pub enum X11Error {
    /// No X server to talk to (no `DISPLAY`, or it refuses the connection).
    #[error("cannot connect to the X server: {0}")]
    Connect(String),
    /// The server has no visual with an alpha channel: the menu could not be
    /// drawn over the screen.
    #[error("the X server has no 32-bit visual with an alpha channel")]
    NoAlphaVisual,
    /// The server refused a request while the window was made.
    #[error("the X server refused a request: {0}")]
    Request(String),
    /// The font of the labels cannot be loaded.
    #[error(transparent)]
    Render(#[from] RenderError),
    /// The thread of the window cannot be started.
    #[error("cannot start the window thread: {0}")]
    Thread(String),
}

impl X11Error {
    fn request(error: impl std::fmt::Display) -> Self {
        Self::Request(error.to_string())
    }
}

/// The window of the menu on X11, running on its own thread.
#[derive(Debug)]
pub struct X11Overlay {
    commands: Sender<Command>,
    waker: Waker,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Wakes the thread of the window, which waits for X events: a message sent
/// to the window itself.
#[derive(Clone)]
struct Waker {
    connection: Arc<RustConnection>,
    window: Window,
    /// Type of the message that wakes the thread: an atom of ours, so that no
    /// other message is taken for a wake-up.
    atom: Atom,
}

impl std::fmt::Debug for Waker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Waker")
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

impl Waker {
    fn wake(&self) {
        let message = ClientMessageEvent::new(32, self.window, self.atom, [0_u32; 5]);
        // A connection that is gone ends the thread by itself.
        let _ = self
            .connection
            .send_event(false, self.window, EventMask::NO_EVENT, message)
            .map(|_| ());
        let _ = self.connection.flush();
    }
}

impl X11Overlay {
    /// Connects to the X server, makes the window (not shown) and starts its
    /// thread, which reports the pointer, the clicks and Escape to `events`.
    ///
    /// # Errors
    ///
    /// [`X11Error`] when there is no X server, it has no alpha visual, or the
    /// thread cannot start.
    pub fn start(events: Report) -> Result<Self, X11Error> {
        let (connection, screen_number) =
            RustConnection::connect(None).map_err(|error| X11Error::Connect(error.to_string()))?;
        let connection = Arc::new(connection);
        let root = connection.setup().roots[screen_number].clone();
        let visual = alpha_visual(&root).ok_or(X11Error::NoAlphaVisual)?;
        let window = make_window(&connection, &root, visual)?;
        let waker = Waker {
            connection: Arc::clone(&connection),
            window,
            atom: intern(&connection, WAKE_ATOM_NAME)?,
        };
        let (commands, received) = mpsc::channel();
        let ticker = Arc::new(Ticker::default());
        let mut state = State {
            connection: Arc::clone(&connection),
            window,
            gc: 0,
            screen: Size {
                width: f64::from(root.width_in_pixels),
                height: f64::from(root.height_in_pixels),
            },
            renderer: TextRenderer::new()?,
            events: Arc::clone(&events),
            wake_atom: waker.atom,
            // Under a Wayland session X clients are XWayland's.
            trust_enter: std::env::var_os("WAYLAND_DISPLAY").is_none(),
            ticker: Arc::clone(&ticker),
            shown: None,
            mapped: false,
            last_pointer: None,
        };
        state.make_gc()?;
        let thread = std::thread::Builder::new()
            .name("radial-overlay-x11".to_owned())
            .spawn(move || {
                // Told when the thread ends, whatever the way (an error, a
                // panic unwinding), and after the window is cleaned up: the
                // host must not go on with a menu that has no window.
                let _gone = Gone(events);
                let _cleanup = Cleanup {
                    connection: Arc::clone(&state.connection),
                    window: state.window,
                    ticker: Arc::clone(&state.ticker),
                };
                if let Err(error) = state.run(&received) {
                    tracing::error!(%error, "the radial menu window stopped");
                }
            })
            .map_err(|error| X11Error::Thread(error.to_string()))?;
        // From here the overlay owns the thread: if the ticker cannot start,
        // dropping the overlay stops it.
        let overlay = Self {
            commands,
            waker,
            thread: Some(thread),
        };
        spawn_ticker(&ticker, &overlay.waker)?;
        Ok(overlay)
    }

    fn send(&self, command: Command) {
        // The thread is gone when it failed: it has said so.
        if self.commands.send(command).is_ok() {
            self.waker.wake();
        }
    }
}

impl Surface for X11Overlay {
    fn open(&self, content: Content) {
        self.send(Command::Open(Box::new(content)));
    }

    fn place(&self, view: View) {
        self.send(Command::Place(view));
    }

    fn redraw(&self, view: View) {
        self.send(Command::Redraw(view));
    }

    fn close(&self) {
        self.send(Command::Close);
    }
}

impl Drop for X11Overlay {
    fn drop(&mut self) {
        self.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// A visual of 32 bits with an alpha channel, `0xAARRGGBB`.
fn alpha_visual(root: &x11rb::protocol::xproto::Screen) -> Option<Visualid> {
    root.allowed_depths
        .iter()
        .filter(|depth| depth.depth == ARGB_DEPTH)
        .flat_map(|depth| &depth.visuals)
        .find(|visual| {
            visual.class == VisualClass::TRUE_COLOR
                && visual.red_mask == RED_MASK
                && visual.green_mask == GREEN_MASK
                && visual.blue_mask == BLUE_MASK
        })
        .map(|visual| visual.visual_id)
}

/// Makes the window, borderless full-screen above the others, with the arrow
/// as its cursor; it is not mapped yet.
fn make_window(
    connection: &RustConnection,
    root: &x11rb::protocol::xproto::Screen,
    visual: Visualid,
) -> Result<Window, X11Error> {
    let window = connection.generate_id().map_err(X11Error::request)?;
    let colormap = connection.generate_id().map_err(X11Error::request)?;
    let cursor = arrow_cursor(connection)?;
    connection
        .create_colormap(ColormapAlloc::NONE, colormap, root.root, visual)
        .map_err(X11Error::request)?;
    let attributes = CreateWindowAux::new()
        .background_pixel(0)
        .border_pixel(0)
        .colormap(colormap)
        .cursor(cursor)
        .event_mask(
            EventMask::EXPOSURE
                | EventMask::POINTER_MOTION
                | EventMask::BUTTON_PRESS
                | EventMask::KEY_PRESS
                | EventMask::ENTER_WINDOW
                | EventMask::STRUCTURE_NOTIFY,
        );
    connection
        .create_window(
            ARGB_DEPTH,
            window,
            root.root,
            0,
            0,
            root.width_in_pixels,
            root.height_in_pixels,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &attributes,
        )
        .map_err(X11Error::request)?;
    set_properties(connection, window)?;
    connection.flush().map_err(X11Error::request)?;
    Ok(window)
}

fn intern(connection: &RustConnection, name: &[u8]) -> Result<Atom, X11Error> {
    connection
        .intern_atom(false, name)
        .map_err(X11Error::request)?
        .reply()
        .map(|reply| reply.atom)
        .map_err(X11Error::request)
}

/// What a thread that ends leaves behind it is taken away, whatever the way it
/// ended: the keyboard grab, the window (a window left mapped would cover the
/// screen and swallow every click), and the ticker.
struct Cleanup {
    connection: Arc<RustConnection>,
    window: Window,
    ticker: Arc<Ticker>,
}

impl Drop for Cleanup {
    fn drop(&mut self) {
        self.ticker.stop();
        // Each fails without harm when the server is gone.
        let _ = self.connection.ungrab_keyboard(CURRENT_TIME);
        let _ = self.connection.unmap_window(self.window);
        let _ = self.connection.destroy_window(self.window);
        let _ = self.connection.flush();
    }
}

/// Name, class and the state the window manager reads at the map: full screen,
/// above the others.
fn set_properties(connection: &RustConnection, window: Window) -> Result<(), X11Error> {
    let atom = |name: &[u8]| intern(connection, name);
    let state = atom(b"_NET_WM_STATE")?;
    let fullscreen = atom(b"_NET_WM_STATE_FULLSCREEN")?;
    let above = atom(b"_NET_WM_STATE_ABOVE")?;
    let skip_taskbar = atom(b"_NET_WM_STATE_SKIP_TASKBAR")?;
    let skip_pager = atom(b"_NET_WM_STATE_SKIP_PAGER")?;
    connection
        .change_property32(
            PropMode::REPLACE,
            window,
            state,
            AtomEnum::ATOM,
            &[fullscreen, above, skip_taskbar, skip_pager],
        )
        .map_err(X11Error::request)?;
    connection
        .change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_NAME,
            AtomEnum::STRING,
            WINDOW_NAME.as_bytes(),
        )
        .map_err(X11Error::request)?;
    connection
        .change_property8(
            PropMode::REPLACE,
            window,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            WINDOW_CLASS,
        )
        .map_err(X11Error::request)?;
    Ok(())
}

/// The plain arrow, from the `cursor` font.
fn arrow_cursor(connection: &RustConnection) -> Result<u32, X11Error> {
    let font = connection.generate_id().map_err(X11Error::request)?;
    let cursor = connection.generate_id().map_err(X11Error::request)?;
    connection
        .open_font(font, b"cursor")
        .map_err(X11Error::request)?;
    connection
        .create_glyph_cursor(
            cursor,
            font,
            font,
            CURSOR_GLYPH_ARROW,
            CURSOR_GLYPH_ARROW + 1,
            CURSOR_FOREGROUND.0,
            CURSOR_FOREGROUND.1,
            CURSOR_FOREGROUND.2,
            CURSOR_BACKGROUND.0,
            CURSOR_BACKGROUND.1,
            CURSOR_BACKGROUND.2,
        )
        .map_err(X11Error::request)?;
    connection.close_font(font).map_err(X11Error::request)?;
    Ok(cursor)
}

/// Drives the frames of the opening and the check of the time the window has
/// been open: a thread that sends a wake-up every frame while the window is
/// shown, and sleeps otherwise.
#[derive(Default)]
struct Ticker {
    shown: Mutex<bool>,
    changed: Condvar,
    stopped: AtomicBool,
}

impl Ticker {
    fn set_shown(&self, shown: bool) {
        *self.shown.lock().unwrap_or_else(PoisonError::into_inner) = shown;
        self.changed.notify_all();
    }

    fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
        self.changed.notify_all();
    }

    /// Waits until the window is shown; `false` when the ticker must end.
    fn wait_until_shown(&self) -> bool {
        let mut shown = self.shown.lock().unwrap_or_else(PoisonError::into_inner);
        while !*shown && !self.stopped.load(Ordering::Relaxed) {
            shown = self
                .changed
                .wait(shown)
                .unwrap_or_else(PoisonError::into_inner);
        }
        !self.stopped.load(Ordering::Relaxed)
    }
}

fn spawn_ticker(ticker: &Arc<Ticker>, waker: &Waker) -> Result<(), X11Error> {
    let (ticker, waker) = (Arc::clone(ticker), waker.clone());
    std::thread::Builder::new()
        .name("radial-overlay-x11-frames".to_owned())
        .spawn(move || {
            while ticker.wait_until_shown() {
                std::thread::sleep(FRAME_PERIOD);
                waker.wake();
            }
        })
        .map(|_| ())
        .map_err(|error| X11Error::Thread(error.to_string()))
}

/// What an X event comes to, once the events another client could have forged
/// are put aside.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Seen {
    Pointer(Point),
    Click,
    Escape,
    Mapped,
    Unmapped,
    Exposed,
    Resized(u16, u16),
    Destroyed,
    /// The message that wakes the thread.
    Wake,
    Ignored,
}

/// What `event` is, `trust_enter` telling whether the position of an enter can
/// be taken for where the pointer is. Another X client can send any event to any window
/// (`SendEvent`), and x11rb hands it over like a real one: only what the server
/// generated counts, and the one message of ours, which is of a type nobody else
/// has a reason to use.
fn interpret(event: &Event, wake: Atom, trust_enter: bool) -> Seen {
    if let Event::ClientMessage(message) = event {
        return if message.type_ == wake {
            Seen::Wake
        } else {
            Seen::Ignored
        };
    }
    if !event.server_generated() {
        return Seen::Ignored;
    }
    match event {
        // The position that comes with the window being shown under the
        // pointer is the right one on Xorg; under XWayland it is the last the X
        // clients saw, which can be anywhere.
        Event::EnterNotify(event) if trust_enter => Seen::Pointer(Point::new(
            f64::from(event.event_x),
            f64::from(event.event_y),
        )),
        Event::MotionNotify(event) => Seen::Pointer(Point::new(
            f64::from(event.event_x),
            f64::from(event.event_y),
        )),
        Event::ButtonPress(_) => Seen::Click,
        Event::KeyPress(event) if event.detail == KEYCODE_ESCAPE => Seen::Escape,
        Event::MapNotify(_) => Seen::Mapped,
        Event::UnmapNotify(_) => Seen::Unmapped,
        Event::Expose(_) => Seen::Exposed,
        Event::ConfigureNotify(event) => Seen::Resized(event.width, event.height),
        Event::DestroyNotify(_) => Seen::Destroyed,
        _ => Seen::Ignored,
    }
}

/// What is on screen.
struct Shown {
    content: Content,
    opened_at: Instant,
    view: Option<View>,
    placed_at: Option<Instant>,
}

struct State {
    connection: Arc<RustConnection>,
    window: Window,
    gc: u32,
    screen: Size,
    renderer: TextRenderer,
    events: Report,
    wake_atom: Atom,
    /// The position of an enter is where the pointer is (Xorg, not `XWayland`).
    trust_enter: bool,
    ticker: Arc<Ticker>,
    shown: Option<Shown>,
    /// The window manager has shown the window (and not taken it away since).
    mapped: bool,
    /// Where the pointer was last reported, to not repeat it.
    last_pointer: Option<Point>,
}

impl State {
    /// The graphics context the menu is drawn with.
    fn make_gc(&mut self) -> Result<(), X11Error> {
        self.gc = self.connection.generate_id().map_err(X11Error::request)?;
        self.connection
            .create_gc(self.gc, self.window, &CreateGCAux::new())
            .map_err(X11Error::request)?;
        Ok(())
    }

    /// Waits for what the host asks (a wake-up) and for X, until told to stop.
    fn run(&mut self, commands: &Receiver<Command>) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            let mut batch = vec![self.connection.wait_for_event()?];
            while batch.len() < MAX_BATCH
                && let Some(event) = self.connection.poll_for_event()?
            {
                batch.push(event);
            }
            let seen: Vec<Seen> = batch
                .iter()
                .map(|event| interpret(event, self.wake_atom, self.trust_enter))
                .collect();
            if !self.handle(seen) {
                return Ok(());
            }
            while let Ok(command) = commands.try_recv() {
                if !self.command(command) {
                    return Ok(());
                }
            }
            self.tick();
        }
    }

    /// `false` when the window is destroyed under us.
    fn handle(&mut self, seen: Vec<Seen>) -> bool {
        let mut pointer = None;
        let mut exposed = false;
        for seen in seen {
            match seen {
                Seen::Pointer(position) => pointer = Some(position),
                Seen::Click if self.shown.is_some() => {
                    self.report_pointer(pointer.take());
                    self.report(WindowEvent::Click);
                }
                Seen::Escape if self.shown.is_some() => self.give_up(),
                Seen::Mapped => self.mapped_by_the_window_manager(),
                Seen::Unmapped => {
                    // Something else took the window away while the menu is
                    // shown: the menu is given up. A late notice of an earlier
                    // closing, with the window not shown again yet, is not.
                    if self.mapped && self.shown.is_some() {
                        self.give_up();
                    }
                    self.mapped = false;
                }
                Seen::Exposed => exposed = true,
                Seen::Resized(width, height) => self.resized(width, height),
                Seen::Destroyed => return false,
                Seen::Click | Seen::Escape | Seen::Wake | Seen::Ignored => {}
            }
        }
        self.report_pointer(pointer);
        if exposed {
            self.draw();
        }
        true
    }

    /// The window manager shows the window when it gets to the request, which
    /// can be after the menu was closed: then it is hidden again at once, with
    /// no grab of the keyboard.
    fn mapped_by_the_window_manager(&mut self) {
        self.mapped = true;
        if self.shown.is_some() {
            self.take_the_keyboard();
            self.draw();
        } else {
            let _ = self.connection.unmap_window(self.window);
            let _ = self.connection.flush();
        }
    }

    /// Escape comes to the window that has the keyboard: the focus when the
    /// window manager gives it, else a grab.
    fn take_the_keyboard(&self) {
        let granted = self
            .connection
            .grab_keyboard(
                true,
                self.window,
                CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|reply| reply.status == GrabStatus::SUCCESS);
        if !granted {
            tracing::debug!("the radial menu window did not get the keyboard grab");
        }
    }

    fn resized(&mut self, width: u16, height: u16) {
        if width == 0
            || height == 0
            || u32::from(width) > MAX_SURFACE_SIDE
            || u32::from(height) > MAX_SURFACE_SIDE
        {
            return;
        }
        let screen = Size {
            width: f64::from(width),
            height: f64::from(height),
        };
        if screen != self.screen {
            self.screen = screen;
            self.report(WindowEvent::Screen(screen));
        }
    }

    /// `false` to stop.
    fn command(&mut self, command: Command) -> bool {
        match command {
            Command::Open(content) => self.show(*content),
            Command::Place(view) => {
                if let Some(shown) = &mut self.shown {
                    shown.view = Some(view);
                    shown.placed_at = Some(Instant::now());
                }
                self.draw();
            }
            Command::Redraw(view) => {
                if let Some(shown) = &mut self.shown {
                    shown.view = Some(view);
                }
                self.draw();
            }
            Command::Close => self.hide(),
            Command::Stop => {
                self.hide();
                return false;
            }
        }
        true
    }

    fn show(&mut self, content: Content) {
        self.shown = Some(Shown {
            content,
            opened_at: Instant::now(),
            view: None,
            placed_at: None,
        });
        self.last_pointer = None;
        self.ticker.set_shown(true);
        // The menu opens where the pointer is: the host learns the size of the
        // screen, then the first position the pointer reports over the window.
        self.report(WindowEvent::Screen(self.screen));
        let mapped = self
            .connection
            .map_window(self.window)
            .and_then(|_| self.connection.flush());
        if let Err(error) = mapped {
            tracing::warn!(%error, "cannot show the radial menu window");
            self.give_up();
        }
    }

    fn hide(&mut self) {
        self.shown = None;
        self.mapped = false;
        self.ticker.set_shown(false);
        // Both fail without harm when the server is gone.
        let _ = self.connection.ungrab_keyboard(CURRENT_TIME);
        let _ = self.connection.unmap_window(self.window);
        let _ = self.connection.flush();
    }

    /// The window goes and the host hears Escape.
    fn give_up(&mut self) {
        self.hide();
        // The host may be gone: nothing to tell then, the window is closed anyway.
        let _ = (self.events)(WindowEvent::Escape);
    }

    /// Frames of the opening, and the longest the window may stay.
    fn tick(&mut self) {
        let Some(shown) = &self.shown else {
            return;
        };
        if has_expired(shown.opened_at, Instant::now()) {
            tracing::warn!(
                limit = ?MAX_OPEN,
                "the radial menu stayed open too long: closed"
            );
            self.give_up();
            return;
        }
        let settling = shown.placed_at.is_some_and(|at| at.elapsed() < SETTLE);
        if settling || opening_progress(&shown.content, shown.placed_at) < 1.0 {
            self.draw();
        }
    }

    fn report(&mut self, event: WindowEvent) {
        if !(self.events)(event) {
            self.hide();
        }
    }

    /// Tells where the pointer is, when it moved.
    fn report_pointer(&mut self, pointer: Option<Point>) {
        if let Some(position) = pointer
            && self.shown.is_some()
            && self.last_pointer != Some(position)
        {
            self.last_pointer = Some(position);
            self.report(WindowEvent::Pointer(position));
        }
    }

    /// Draws the menu at its place: only the square it is in is sent, the
    /// rest of the window stays transparent.
    fn draw(&self) {
        let Some(shown) = &self.shown else {
            return;
        };
        let Some(view) = shown.view else {
            return;
        };
        let petals: Vec<PetalView<'_>> = shown
            .content
            .petals
            .iter()
            .map(|petal| PetalView {
                label: &petal.label,
                icon: petal.icon.as_ref(),
            })
            .collect();
        let image = match paint(
            &self.renderer,
            &shown.content.layout,
            &petals,
            view.lit,
            opening_progress(&shown.content, shown.placed_at),
            shown.content.palette,
        ) {
            Ok(image) => image,
            Err(error) => {
                tracing::warn!(%error, "cannot draw the radial menu");
                return;
            }
        };
        let (width, height) = (image.width(), image.height());
        let (Ok(wide), Ok(high)) = (u16::try_from(width), u16::try_from(height)) else {
            return;
        };
        let left = offset(view.centre.x, width);
        let top = offset(view.centre.y, height);
        let sent = self
            .connection
            .put_image(
                ImageFormat::Z_PIXMAP,
                self.window,
                self.gc,
                wide,
                high,
                left,
                top,
                0,
                ARGB_DEPTH,
                &premultiplied_bgra(&image),
            )
            .and_then(|_| self.connection.flush());
        if let Err(error) = sent {
            tracing::debug!(%error, "cannot send the radial menu image");
        }
    }
}

/// Where the top left of an image of `length` pixels is, to be centred on
/// `centre`; cut to what X11 positions can hold.
fn offset(centre: f64, length: u32) -> i16 {
    #[allow(clippy::cast_possible_truncation)]
    let corner = (centre - f64::from(length) / 2.0).round() as i64;
    i16::try_from(corner).unwrap_or(if corner < 0 { i16::MIN } else { i16::MAX })
}

#[cfg(test)]
mod tests {
    use x11rb::protocol::xproto::{Depth, Screen, Visualtype};

    use super::*;

    fn visual(id: u32, class: VisualClass, masks: (u32, u32, u32)) -> Visualtype {
        Visualtype {
            visual_id: id,
            class,
            bits_per_rgb_value: 8,
            colormap_entries: 256,
            red_mask: masks.0,
            green_mask: masks.1,
            blue_mask: masks.2,
        }
    }

    fn screen(depths: Vec<Depth>) -> Screen {
        Screen {
            allowed_depths: depths,
            ..Screen::default()
        }
    }

    const ARGB: (u32, u32, u32) = (RED_MASK, GREEN_MASK, BLUE_MASK);

    #[test]
    fn the_alpha_visual_is_the_32_bit_true_color_one_with_the_usual_masks() {
        let depths = vec![
            Depth {
                depth: 24,
                visuals: vec![visual(1, VisualClass::TRUE_COLOR, ARGB)],
            },
            Depth {
                depth: 32,
                visuals: vec![
                    visual(2, VisualClass::DIRECT_COLOR, ARGB),
                    visual(3, VisualClass::TRUE_COLOR, (0xff, 0xff00, 0x00ff_0000)),
                    visual(4, VisualClass::TRUE_COLOR, ARGB),
                ],
            },
        ];
        assert_eq!(alpha_visual(&screen(depths)), Some(4));
    }

    #[test]
    fn a_server_without_a_32_bit_visual_gives_none() {
        let depths = vec![Depth {
            depth: 24,
            visuals: vec![visual(1, VisualClass::TRUE_COLOR, ARGB)],
        }];
        assert_eq!(alpha_visual(&screen(depths)), None);
        assert_eq!(alpha_visual(&screen(Vec::new())), None);
    }

    /// The 32 bytes of an event of `kind`, with the pointer or key fields given.
    fn raw_event(kind: u8, detail: u8, x: i16, y: i16) -> [u8; 32] {
        let mut bytes = [0_u8; 32];
        bytes[0] = kind;
        bytes[1] = detail;
        bytes[24..26].copy_from_slice(&x.to_ne_bytes());
        bytes[26..28].copy_from_slice(&y.to_ne_bytes());
        bytes
    }

    fn parsed(raw: &[u8; 32]) -> Event {
        use x11rb::protocol::xproto::{ButtonPressEvent, KeyPressEvent, MotionNotifyEvent};
        use x11rb::x11_utils::TryParse;
        match raw[0] & 0x7f {
            2 => Event::KeyPress(KeyPressEvent::try_parse(raw).unwrap().0),
            4 => Event::ButtonPress(ButtonPressEvent::try_parse(raw).unwrap().0),
            6 => Event::MotionNotify(MotionNotifyEvent::try_parse(raw).unwrap().0),
            other => panic!("not an event of this test: {other}"),
        }
    }

    const WAKE: Atom = 777;
    const KEY_PRESS: u8 = 2;
    const BUTTON_PRESS: u8 = 4;
    const MOTION_NOTIFY: u8 = 6;
    const SENT_BY_A_CLIENT: u8 = 0x80;

    #[test]
    fn what_the_server_generates_is_taken() {
        let motion = parsed(&raw_event(MOTION_NOTIFY, 0, 120, 340));
        assert_eq!(
            interpret(&motion, WAKE, true),
            Seen::Pointer(Point::new(120.0, 340.0))
        );
        let click = parsed(&raw_event(BUTTON_PRESS, 1, 0, 0));
        assert_eq!(interpret(&click, WAKE, true), Seen::Click);
        let escape = parsed(&raw_event(KEY_PRESS, KEYCODE_ESCAPE, 0, 0));
        assert_eq!(interpret(&escape, WAKE, true), Seen::Escape);
        let other_key = parsed(&raw_event(KEY_PRESS, KEYCODE_ESCAPE + 1, 0, 0));
        assert_eq!(interpret(&other_key, WAKE, true), Seen::Ignored);
    }

    #[test]
    fn what_another_client_sends_is_ignored() {
        // The same events, with the bit `SendEvent` sets: a click that would
        // choose a petal, a motion that would light one, an Escape.
        for (kind, detail) in [
            (MOTION_NOTIFY, 0),
            (BUTTON_PRESS, 1),
            (KEY_PRESS, KEYCODE_ESCAPE),
        ] {
            let forged = parsed(&raw_event(kind | SENT_BY_A_CLIENT, detail, 10, 10));
            assert_eq!(interpret(&forged, WAKE, true), Seen::Ignored, "kind {kind}");
        }
    }

    #[test]
    fn the_position_of_an_enter_is_taken_only_where_it_can_be_trusted() {
        use x11rb::protocol::xproto::EnterNotifyEvent;
        use x11rb::x11_utils::TryParse;
        const ENTER_NOTIFY: u8 = 7;
        let enter = Event::EnterNotify(
            EnterNotifyEvent::try_parse(&raw_event(ENTER_NOTIFY, 0, 55, 66))
                .unwrap()
                .0,
        );
        assert_eq!(
            interpret(&enter, WAKE, true),
            Seen::Pointer(Point::new(55.0, 66.0))
        );
        assert_eq!(interpret(&enter, WAKE, false), Seen::Ignored);
    }

    #[test]
    fn only_the_message_of_our_own_type_wakes_the_thread() {
        use x11rb::protocol::xproto::ClientMessageEvent;
        let ours = Event::ClientMessage(ClientMessageEvent::new(32, 1, WAKE, [0_u32; 5]));
        let theirs = Event::ClientMessage(ClientMessageEvent::new(32, 1, WAKE + 1, [0_u32; 5]));
        assert_eq!(interpret(&ours, WAKE, true), Seen::Wake);
        assert_eq!(interpret(&theirs, WAKE, true), Seen::Ignored);
    }

    #[test]
    fn an_image_is_centred_on_its_point_and_cut_to_what_x11_positions_hold() {
        assert_eq!(offset(100.0, 20), 90);
        assert_eq!(offset(5.0, 20), -5);
        assert_eq!(offset(1.0e9, 20), i16::MAX);
        assert_eq!(offset(-1.0e9, 20), i16::MIN);
    }
}
