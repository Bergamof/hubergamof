//! Simulated devices for `--demo`, UI development and tests.
//!
//! A [`VirtualDeck`] keeps the last image written to each key and the whole
//! touch strip in memory (exposed later as live state) and lets callers inject
//! input events (UI `pressKey`/`rotateDial`/`pressDial`, tests).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use hubergamof_protocol::{DeviceCapabilities, DeviceInfo, DeviceModel, ImageSize, MAX_BRIGHTNESS};
use image::{RgbaImage, imageops};
use tokio::sync::broadcast;

use super::strip::{check_image_size, check_strip_region};
use super::{DeckBackend, DeviceDiscovery, DeviceError, INPUT_EVENT_CAPACITY, InputEvent};

/// Serial of the device attached by `--demo`.
pub const DEMO_SERIAL: &str = "DEMO0000001";

/// Brightness of a virtual device before any `set_brightness`, in percent.
pub const VIRTUAL_DEFAULT_BRIGHTNESS: u8 = 100;

const VIRTUAL_PLUS_NAME: &str = "Stream Deck+ (virtual)";

/// Capabilities of a Stream Deck+: 4×2 keys of 120×120, 4 dials, 800×100 strip.
pub const STREAM_DECK_PLUS_CAPABILITIES: DeviceCapabilities = DeviceCapabilities {
    key_columns: 4,
    key_rows: 2,
    key_image_size: ImageSize {
        width: 120,
        height: 120,
    },
    dial_count: 4,
    touch_strip: Some(ImageSize {
        width: 800,
        height: 100,
    }),
};

/// Description of a virtual Stream Deck+ with the given serial.
#[must_use]
pub fn virtual_plus_info(serial: &str) -> DeviceInfo {
    DeviceInfo {
        serial: serial.to_owned(),
        model: DeviceModel::Virtual,
        display_name: VIRTUAL_PLUS_NAME.to_owned(),
        capabilities: STREAM_DECK_PLUS_CAPABILITIES,
    }
}

/// What a virtual device currently displays.
#[derive(Debug, Clone)]
pub struct VirtualScreen {
    /// Last image of each key, `None` when blank.
    pub keys: Vec<Option<Arc<RgbaImage>>>,
    /// Whole touch strip, `None` when the device has no strip.
    pub strip: Option<Arc<RgbaImage>>,
    /// Brightness in percent.
    pub brightness: u8,
}

impl VirtualScreen {
    fn blank(capabilities: &DeviceCapabilities) -> Self {
        Self {
            keys: vec![None; capabilities.key_count()],
            strip: capabilities
                .touch_strip
                .map(|size| Arc::new(RgbaImage::new(size.width, size.height))),
            brightness: VIRTUAL_DEFAULT_BRIGHTNESS,
        }
    }
}

/// A simulated device.
pub struct VirtualDeck {
    info: DeviceInfo,
    events: broadcast::Sender<InputEvent>,
    screen: Mutex<VirtualScreen>,
    connected: AtomicBool,
}

impl VirtualDeck {
    /// Creates a connected virtual device.
    #[must_use]
    pub fn new(info: DeviceInfo) -> Self {
        let (events, _) = broadcast::channel(INPUT_EVENT_CAPACITY);
        let screen = VirtualScreen::blank(&info.capabilities);
        Self {
            info,
            events,
            screen: Mutex::new(screen),
            connected: AtomicBool::new(true),
        }
    }

    /// Copy of what the device currently displays.
    #[must_use]
    pub fn screen(&self) -> VirtualScreen {
        self.lock_screen().clone()
    }

    /// Last image written to key `index`.
    #[must_use]
    pub fn key_image(&self, index: u8) -> Option<Arc<RgbaImage>> {
        self.lock_screen()
            .keys
            .get(usize::from(index))
            .cloned()
            .flatten()
    }

    /// Injects an input event, as if it came from the hardware.
    ///
    /// # Errors
    ///
    /// [`DeviceError::Disconnected`] after [`DeckBackend::close`];
    /// [`DeviceError::InvalidKey`] / [`DeviceError::InvalidDial`] /
    /// [`DeviceError::NoTouchStrip`] when the event does not exist on this
    /// device.
    pub fn inject(&self, event: InputEvent) -> Result<(), DeviceError> {
        self.ensure_connected()?;
        self.check_event(event)?;
        // An error only means there is no subscriber right now.
        let _ = self.events.send(event);
        Ok(())
    }

    fn check_event(&self, event: InputEvent) -> Result<(), DeviceError> {
        let capabilities = &self.info.capabilities;
        match event {
            InputEvent::KeyDown(index) | InputEvent::KeyUp(index) => self.check_key(index),
            InputEvent::DialDown(index)
            | InputEvent::DialUp(index)
            | InputEvent::DialRotate { index, .. } => {
                if index < capabilities.dial_count {
                    Ok(())
                } else {
                    Err(DeviceError::InvalidDial {
                        index,
                        count: capabilities.dial_count,
                    })
                }
            }
            InputEvent::TouchTap { .. } | InputEvent::TouchSwipe { .. } => capabilities
                .touch_strip
                .map(|_| ())
                .ok_or(DeviceError::NoTouchStrip),
            InputEvent::Disconnected => Ok(()),
        }
    }

    fn check_key(&self, index: u8) -> Result<(), DeviceError> {
        let count = self.info.capabilities.key_count();
        if usize::from(index) < count {
            Ok(())
        } else {
            Err(DeviceError::InvalidKey { index, count })
        }
    }

    fn ensure_connected(&self) -> Result<(), DeviceError> {
        if self.is_connected() {
            Ok(())
        } else {
            Err(DeviceError::Disconnected)
        }
    }

    fn lock_screen(&self) -> MutexGuard<'_, VirtualScreen> {
        // Every update replaces whole values: a poisoned lock is still valid.
        self.screen.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl DeckBackend for VirtualDeck {
    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn subscribe(&self) -> broadcast::Receiver<InputEvent> {
        self.events.subscribe()
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }

    async fn set_key_image(&self, index: u8, image: Arc<RgbaImage>) -> Result<(), DeviceError> {
        self.ensure_connected()?;
        self.check_key(index)?;
        check_image_size(&image, self.info.capabilities.key_image_size)?;
        self.lock_screen().keys[usize::from(index)] = Some(image);
        Ok(())
    }

    async fn set_strip_region(&self, x: u32, image: Arc<RgbaImage>) -> Result<(), DeviceError> {
        self.ensure_connected()?;
        let size = self
            .info
            .capabilities
            .touch_strip
            .ok_or(DeviceError::NoTouchStrip)?;
        check_strip_region(size, x, &image)?;
        let mut screen = self.lock_screen();
        if let Some(strip) = screen.strip.as_mut() {
            imageops::replace(Arc::make_mut(strip), image.as_ref(), i64::from(x), 0);
        }
        Ok(())
    }

    async fn set_brightness(&self, percent: u8) -> Result<(), DeviceError> {
        self.ensure_connected()?;
        if percent > MAX_BRIGHTNESS {
            return Err(DeviceError::BrightnessOutOfRange(percent));
        }
        self.lock_screen().brightness = percent;
        Ok(())
    }

    async fn clear_all(&self) -> Result<(), DeviceError> {
        self.ensure_connected()?;
        let mut screen = self.lock_screen();
        let brightness = screen.brightness;
        *screen = VirtualScreen {
            brightness,
            ..VirtualScreen::blank(&self.info.capabilities)
        };
        Ok(())
    }

    async fn flush(&self) -> Result<(), DeviceError> {
        self.ensure_connected()
    }

    async fn close(&self) {
        if self.connected.swap(false, Ordering::AcqRel) {
            let _ = self.events.send(InputEvent::Disconnected);
        }
    }
}

/// Source of virtual devices feeding a [`DeviceDiscovery`].
#[derive(Clone)]
pub struct VirtualDevices {
    discovery: DeviceDiscovery,
    decks: Arc<Mutex<BTreeMap<String, Arc<VirtualDeck>>>>,
}

impl VirtualDevices {
    /// Creates a source with no device attached.
    #[must_use]
    pub fn new(discovery: DeviceDiscovery) -> Self {
        Self {
            discovery,
            decks: Arc::default(),
        }
    }

    /// Plugs a virtual device in and returns it.
    #[must_use = "the returned deck is the handle used to inject input"]
    pub fn attach(&self, info: DeviceInfo) -> Arc<VirtualDeck> {
        let deck = Arc::new(VirtualDeck::new(info));
        self.lock_decks()
            .insert(deck.info().serial.clone(), Arc::clone(&deck));
        self.discovery
            .attach(Arc::clone(&deck) as Arc<dyn DeckBackend>);
        deck
    }

    /// Unplugs a virtual device; returns whether it was attached.
    pub async fn detach(&self, serial: &str) -> bool {
        let removed = self.lock_decks().remove(serial);
        match removed {
            Some(deck) => {
                deck.close().await;
                self.discovery.detach(serial);
                true
            }
            None => false,
        }
    }

    /// The virtual device with this serial, with its simulation API.
    #[must_use]
    pub fn get(&self, serial: &str) -> Option<Arc<VirtualDeck>> {
        self.lock_decks().get(serial).cloned()
    }

    fn lock_decks(&self) -> MutexGuard<'_, BTreeMap<String, Arc<VirtualDeck>>> {
        self.decks.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::DeviceModel;
    use image::Rgba;

    use super::*;
    use crate::device::{DiscoveryEvent, TouchPoint};

    fn plus() -> VirtualDeck {
        VirtualDeck::new(virtual_plus_info(DEMO_SERIAL))
    }

    #[test]
    fn virtual_plus_has_stream_deck_plus_capabilities() {
        let info = virtual_plus_info(DEMO_SERIAL);
        assert_eq!(info.model, DeviceModel::Virtual);
        assert_eq!(info.capabilities.key_count(), 8);
        assert_eq!(info.capabilities.dial_count, 4);
        assert_eq!(
            info.capabilities.touch_strip,
            Some(ImageSize {
                width: 800,
                height: 100
            })
        );
    }

    #[tokio::test]
    async fn key_images_are_kept_in_memory() {
        let deck = plus();
        let image = RgbaImage::from_pixel(120, 120, Rgba([255, 0, 0, 255]));
        deck.set_key_image(3, Arc::new(image.clone()))
            .await
            .unwrap();
        assert_eq!(*deck.key_image(3).unwrap(), image);
        assert!(deck.key_image(2).is_none());
    }

    #[tokio::test]
    async fn invalid_key_writes_are_rejected() {
        let deck = plus();
        assert!(matches!(
            deck.set_key_image(8, Arc::new(RgbaImage::new(120, 120)))
                .await,
            Err(DeviceError::InvalidKey { index: 8, count: 8 })
        ));
        assert!(matches!(
            deck.set_key_image(0, Arc::new(RgbaImage::new(100, 100)))
                .await,
            Err(DeviceError::ImageSizeMismatch { .. })
        ));
    }

    #[tokio::test]
    async fn touch_zone_images_land_in_their_zone() {
        let deck = plus();
        let red = Rgba([255, 0, 0, 255]);
        deck.set_touch_zone_image(2, Arc::new(RgbaImage::from_pixel(200, 100, red)))
            .await
            .unwrap();
        let strip = deck.screen().strip.unwrap();
        assert_eq!(*strip.get_pixel(399, 50), Rgba([0, 0, 0, 0]));
        assert_eq!(*strip.get_pixel(400, 50), red);
        assert_eq!(*strip.get_pixel(599, 99), red);
        assert_eq!(*strip.get_pixel(600, 0), Rgba([0, 0, 0, 0]));
        assert!(matches!(
            deck.set_touch_zone_image(4, Arc::new(RgbaImage::new(200, 100)))
                .await,
            Err(DeviceError::InvalidZone { .. })
        ));
    }

    #[tokio::test]
    async fn clear_all_blanks_keys_and_strip_but_keeps_brightness() {
        let deck = plus();
        deck.set_brightness(40).await.unwrap();
        deck.set_key_image(
            0,
            Arc::new(RgbaImage::from_pixel(120, 120, Rgba([1, 2, 3, 255]))),
        )
        .await
        .unwrap();
        deck.clear_all().await.unwrap();
        let screen = deck.screen();
        assert!(screen.keys.iter().all(Option::is_none));
        assert_eq!(screen.brightness, 40);
        assert!(matches!(
            deck.set_brightness(101).await,
            Err(DeviceError::BrightnessOutOfRange(101))
        ));
    }

    #[tokio::test]
    async fn injected_events_reach_subscribers() {
        let deck = plus();
        let mut events = deck.subscribe();
        let tap = InputEvent::TouchTap {
            point: TouchPoint { x: 10, y: 20 },
            long: false,
        };
        deck.inject(InputEvent::KeyDown(7)).unwrap();
        deck.inject(InputEvent::DialRotate {
            index: 3,
            ticks: -2,
        })
        .unwrap();
        deck.inject(tap).unwrap();
        assert_eq!(events.recv().await.unwrap(), InputEvent::KeyDown(7));
        assert_eq!(
            events.recv().await.unwrap(),
            InputEvent::DialRotate {
                index: 3,
                ticks: -2
            }
        );
        assert_eq!(events.recv().await.unwrap(), tap);
    }

    #[test]
    fn injecting_nonexistent_inputs_is_rejected() {
        let deck = plus();
        assert!(matches!(
            deck.inject(InputEvent::KeyUp(8)),
            Err(DeviceError::InvalidKey { .. })
        ));
        assert!(matches!(
            deck.inject(InputEvent::DialDown(4)),
            Err(DeviceError::InvalidDial { index: 4, count: 4 })
        ));
    }

    #[tokio::test]
    async fn closed_device_emits_disconnected_and_refuses_calls() {
        let deck = plus();
        let mut events = deck.subscribe();
        deck.close().await;
        deck.close().await;
        assert_eq!(events.recv().await.unwrap(), InputEvent::Disconnected);
        assert!(events.try_recv().is_err());
        assert!(!deck.is_connected());
        assert!(matches!(deck.flush().await, Err(DeviceError::Disconnected)));
        assert!(matches!(
            deck.inject(InputEvent::KeyDown(0)),
            Err(DeviceError::Disconnected)
        ));
    }

    #[tokio::test]
    async fn multiple_devices_attach_and_detach() {
        let discovery = DeviceDiscovery::new();
        let source = VirtualDevices::new(discovery.clone());
        let (initial, mut events) = discovery.subscribe();
        assert_eq!(initial, []);

        let first = source.attach(virtual_plus_info("A"));
        let _ = source.attach(virtual_plus_info("B"));
        assert_eq!(
            events.recv().await.unwrap(),
            DiscoveryEvent::Attached(virtual_plus_info("A"))
        );
        assert_eq!(
            events.recv().await.unwrap(),
            DiscoveryEvent::Attached(virtual_plus_info("B"))
        );
        let serials: Vec<_> = discovery.devices().into_iter().map(|d| d.serial).collect();
        assert_eq!(serials, ["A", "B"]);

        let mut first_inputs = first.subscribe();
        assert!(source.detach("A").await);
        assert!(!source.detach("A").await);
        assert_eq!(
            events.recv().await.unwrap(),
            DiscoveryEvent::Detached("A".into())
        );
        assert_eq!(first_inputs.recv().await.unwrap(), InputEvent::Disconnected);
        assert!(discovery.backend("A").is_none());
        assert!(source.get("A").is_none());
        assert!(discovery.backend("B").is_some());
        assert!(source.get("B").is_some());
    }

    #[test]
    fn subscribe_snapshot_includes_devices_attached_before() {
        let discovery = DeviceDiscovery::new();
        let source = VirtualDevices::new(discovery.clone());
        let _ = source.attach(virtual_plus_info(DEMO_SERIAL));
        let (snapshot, mut events) = discovery.subscribe();
        assert_eq!(snapshot, [virtual_plus_info(DEMO_SERIAL)]);
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn reattaching_a_serial_reports_detach_then_attach() {
        let discovery = DeviceDiscovery::new();
        let source = VirtualDevices::new(discovery.clone());
        let _ = source.attach(virtual_plus_info("A"));
        let (_, mut events) = discovery.subscribe();
        let _ = source.attach(virtual_plus_info("A"));
        assert_eq!(
            events.try_recv().unwrap(),
            DiscoveryEvent::Detached("A".into())
        );
        assert_eq!(
            events.try_recv().unwrap(),
            DiscoveryEvent::Attached(virtual_plus_info("A"))
        );
    }
}
