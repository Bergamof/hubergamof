//! A real device driven by a dedicated OS thread.
//!
//! `hidapi` calls block and `HidDevice` is not `Sync`, so each device is owned
//! by one thread that alternates between short input reads and the commands
//! queued by the async side; replies travel back on oneshot channels so the
//! tokio runtime is never blocked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use async_trait::async_trait;
use bergadeck_protocol::{DeviceInfo, MAX_BRIGHTNESS};
use elgato_streamdeck::images::ImageRect;
use elgato_streamdeck::{StreamDeck, StreamDeckError};
use image::{DynamicImage, RgbaImage};
use tokio::sync::{broadcast, oneshot};

use super::mapping::InputTracker;
use crate::device::strip::{check_image_size, check_strip_region};
use crate::device::{DeckBackend, DeviceError, INPUT_EVENT_CAPACITY, InputEvent};

/// How long one input read waits for data before queued commands are
/// processed; bounds the latency of image writes.
pub const INPUT_READ_TIMEOUT: Duration = Duration::from_millis(10);

type Reply = oneshot::Sender<Result<(), DeviceError>>;

enum Command {
    SetKeyImage {
        index: u8,
        image: RgbaImage,
        reply: Reply,
    },
    SetStripRegion {
        x: u16,
        image: RgbaImage,
        reply: Reply,
    },
    SetBrightness {
        percent: u8,
        reply: Reply,
    },
    ClearAll {
        reply: Reply,
    },
    Flush {
        reply: Reply,
    },
    Close {
        reply: oneshot::Sender<()>,
    },
}

/// A connected hardware Stream Deck.
pub struct HardwareDeck {
    info: DeviceInfo,
    events: broadcast::Sender<InputEvent>,
    commands: mpsc::Sender<Command>,
    connected: Arc<AtomicBool>,
}

impl HardwareDeck {
    /// Starts the worker thread owning `deck`.
    ///
    /// # Errors
    ///
    /// [`DeviceError::WorkerSpawn`] when the thread cannot be created.
    pub fn start(deck: StreamDeck, info: DeviceInfo) -> Result<Self, DeviceError> {
        let (events, _) = broadcast::channel(INPUT_EVENT_CAPACITY);
        let (commands, receiver) = mpsc::channel();
        let connected = Arc::new(AtomicBool::new(true));
        let worker = Worker {
            tracker: InputTracker::new(info.capabilities.key_count()),
            strip_size: info.capabilities.touch_strip,
            deck,
            commands: receiver,
            events: events.clone(),
            connected: Arc::clone(&connected),
        };
        thread::Builder::new()
            .name(format!("deck-{}", info.serial))
            .spawn(move || worker.run())
            .map_err(DeviceError::WorkerSpawn)?;
        Ok(Self {
            info,
            events,
            commands,
            connected,
        })
    }

    async fn request(&self, command: impl FnOnce(Reply) -> Command) -> Result<(), DeviceError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(command(reply))
            .map_err(|_| DeviceError::Disconnected)?;
        response.await.map_err(|_| DeviceError::Disconnected)?
    }
}

#[async_trait]
impl DeckBackend for HardwareDeck {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn subscribe(&self) -> broadcast::Receiver<InputEvent> {
        self.events.subscribe()
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    async fn set_key_image(&self, index: u8, image: RgbaImage) -> Result<(), DeviceError> {
        let count = self.info.capabilities.key_count();
        if usize::from(index) >= count {
            return Err(DeviceError::InvalidKey { index, count });
        }
        check_image_size(&image, self.info.capabilities.key_image_size)?;
        self.request(|reply| Command::SetKeyImage {
            index,
            image,
            reply,
        })
        .await
    }

    async fn set_strip_region(&self, x: u32, image: RgbaImage) -> Result<(), DeviceError> {
        let size = self
            .info
            .capabilities
            .touch_strip
            .ok_or(DeviceError::NoTouchStrip)?;
        check_strip_region(size, x, &image)?;
        let x = u16::try_from(x).map_err(|_| DeviceError::CoordinateOutOfRange(x))?;
        self.request(|reply| Command::SetStripRegion { x, image, reply })
            .await
    }

    async fn set_brightness(&self, percent: u8) -> Result<(), DeviceError> {
        if percent > MAX_BRIGHTNESS {
            return Err(DeviceError::BrightnessOutOfRange(percent));
        }
        self.request(|reply| Command::SetBrightness { percent, reply })
            .await
    }

    async fn clear_all(&self) -> Result<(), DeviceError> {
        self.request(|reply| Command::ClearAll { reply }).await
    }

    async fn flush(&self) -> Result<(), DeviceError> {
        self.request(|reply| Command::Flush { reply }).await
    }

    async fn close(&self) {
        let (reply, done) = oneshot::channel();
        if self.commands.send(Command::Close { reply }).is_ok() {
            // An error means the worker already stopped on its own.
            let _ = done.await;
        }
    }
}

struct Worker {
    deck: StreamDeck,
    tracker: InputTracker,
    strip_size: Option<bergadeck_protocol::ImageSize>,
    commands: mpsc::Receiver<Command>,
    events: broadcast::Sender<InputEvent>,
    connected: Arc<AtomicBool>,
}

enum Flow {
    Continue,
    Stop,
}

impl Worker {
    fn run(mut self) {
        while let Flow::Continue = self.process_commands() {
            if let Flow::Stop = self.read_input() {
                break;
            }
        }
        self.connected.store(false, Ordering::Release);
        let _ = self.events.send(InputEvent::Disconnected);
        tracing::debug!("device worker stopped");
    }

    fn process_commands(&self) -> Flow {
        loop {
            match self.commands.try_recv() {
                Ok(Command::Close { reply }) => {
                    // Stop before acknowledging so that `close` returns once
                    // `is_connected` is false.
                    self.connected.store(false, Ordering::Release);
                    let _ = reply.send(());
                    return Flow::Stop;
                }
                Ok(command) => self.execute(command),
                Err(mpsc::TryRecvError::Empty) => return Flow::Continue,
                Err(mpsc::TryRecvError::Disconnected) => return Flow::Stop,
            }
        }
    }

    fn execute(&self, command: Command) {
        let (result, reply) = match command {
            Command::SetKeyImage {
                index,
                image,
                reply,
            } => (
                self.deck
                    .set_button_image(index, DynamicImage::ImageRgba8(image)),
                reply,
            ),
            Command::SetStripRegion { x, image, reply } => (self.write_strip(x, image), reply),
            Command::SetBrightness { percent, reply } => (self.deck.set_brightness(percent), reply),
            Command::ClearAll { reply } => (self.clear_all(), reply),
            Command::Flush { reply } => (self.deck.flush(), reply),
            Command::Close { .. } => return,
        };
        // An error means the caller stopped waiting.
        let _ = reply.send(result.map_err(DeviceError::from));
    }

    fn write_strip(&self, x: u16, image: RgbaImage) -> Result<(), StreamDeckError> {
        let rect = ImageRect::from_image(DynamicImage::ImageRgba8(image))?;
        self.deck.write_lcd(x, 0, &rect)
    }

    fn clear_all(&self) -> Result<(), StreamDeckError> {
        self.deck.clear_all_button_images()?;
        match self.strip_size {
            Some(size) => self.write_strip(0, RgbaImage::new(size.width, size.height)),
            None => Ok(()),
        }
    }

    fn read_input(&mut self) -> Flow {
        match self.deck.read_input(Some(INPUT_READ_TIMEOUT)) {
            Ok(input) => {
                for event in self.tracker.apply(input) {
                    // An error only means there is no subscriber right now.
                    let _ = self.events.send(event);
                }
                Flow::Continue
            }
            Err(StreamDeckError::HidError(error)) => {
                tracing::info!(%error, "device read failed, treating it as unplugged");
                Flow::Stop
            }
            Err(error) => {
                tracing::warn!(%error, "ignoring malformed device report");
                Flow::Continue
            }
        }
    }
}
