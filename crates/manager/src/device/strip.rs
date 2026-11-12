//! Touch strip geometry.

use hubergamof_protocol::{DeviceInfo, ImageSize};
use image::RgbaImage;

use super::DeviceError;

/// The touch strip of a device: it is drawn as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StripLayout {
    strip: ImageSize,
}

impl StripLayout {
    /// Layout of the device touch strip, `None` when the device has no strip
    /// or no dial to remind under it.
    #[must_use]
    pub fn for_device(info: &DeviceInfo) -> Option<Self> {
        let capabilities = &info.capabilities;
        let strip = capabilities.touch_strip?;
        (capabilities.dial_count > 0).then_some(Self { strip })
    }

    /// Size of the whole strip.
    #[must_use]
    pub fn strip_size(&self) -> ImageSize {
        self.strip
    }
}

/// Checks that `image` has exactly the `expected` size.
///
/// # Errors
///
/// [`DeviceError::ImageSizeMismatch`] otherwise.
pub(crate) fn check_image_size(image: &RgbaImage, expected: ImageSize) -> Result<(), DeviceError> {
    if image.width() == expected.width && image.height() == expected.height {
        Ok(())
    } else {
        Err(DeviceError::ImageSizeMismatch {
            expected_width: expected.width,
            expected_height: expected.height,
            actual_width: image.width(),
            actual_height: image.height(),
        })
    }
}

/// Checks that a region of `width` starting at `x` fits in `strip`, and that
/// the region has the strip height.
///
/// # Errors
///
/// [`DeviceError::RegionOutOfBounds`] or [`DeviceError::ImageSizeMismatch`].
pub(crate) fn check_strip_region(
    strip: ImageSize,
    x: u32,
    image: &RgbaImage,
) -> Result<(), DeviceError> {
    let fits = x
        .checked_add(image.width())
        .is_some_and(|right| right <= strip.width);
    if !fits {
        return Err(DeviceError::RegionOutOfBounds {
            x,
            width: image.width(),
        });
    }
    check_image_size(
        image,
        ImageSize {
            width: image.width(),
            height: strip.height,
        },
    )
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::{DeviceCapabilities, DeviceModel};

    use super::*;

    fn info(dial_count: u8, touch_strip: Option<ImageSize>) -> DeviceInfo {
        DeviceInfo {
            serial: "S".into(),
            model: DeviceModel::Plus,
            display_name: "Deck".into(),
            capabilities: DeviceCapabilities {
                key_columns: 4,
                key_rows: 2,
                key_image_size: ImageSize {
                    width: 120,
                    height: 120,
                },
                dial_count,
                touch_strip,
            },
        }
    }

    const PLUS_STRIP: ImageSize = ImageSize {
        width: 800,
        height: 100,
    };

    #[test]
    fn plus_strip_is_800_by_100() {
        let layout = StripLayout::for_device(&info(4, Some(PLUS_STRIP))).unwrap();
        assert_eq!(layout.strip_size(), PLUS_STRIP);
    }

    #[test]
    fn devices_without_strip_or_dials_have_no_layout() {
        assert!(StripLayout::for_device(&info(4, None)).is_none());
        assert!(StripLayout::for_device(&info(0, Some(PLUS_STRIP))).is_none());
    }

    #[test]
    fn strip_region_bounds_are_checked() {
        let image = RgbaImage::new(200, 100);
        assert!(check_strip_region(PLUS_STRIP, 600, &image).is_ok());
        assert!(matches!(
            check_strip_region(PLUS_STRIP, 601, &image),
            Err(DeviceError::RegionOutOfBounds { .. })
        ));
        assert!(matches!(
            check_strip_region(PLUS_STRIP, u32::MAX, &image),
            Err(DeviceError::RegionOutOfBounds { .. })
        ));
        assert!(matches!(
            check_strip_region(PLUS_STRIP, 0, &RgbaImage::new(200, 50)),
            Err(DeviceError::ImageSizeMismatch { .. })
        ));
    }
}
