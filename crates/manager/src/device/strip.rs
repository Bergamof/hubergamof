//! Touch strip geometry: one zone of equal width above each dial.

use hubergamof_protocol::{DeviceInfo, ImageSize};
use image::RgbaImage;

use super::DeviceError;

/// Split of a touch strip into one zone per dial.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StripLayout {
    strip: ImageSize,
    zone_count: u8,
}

impl StripLayout {
    /// Layout of the device touch strip, `None` when the device has no strip
    /// or no dial to split it by.
    #[must_use]
    pub fn for_device(info: &DeviceInfo) -> Option<Self> {
        let capabilities = &info.capabilities;
        let strip = capabilities.touch_strip?;
        (capabilities.dial_count > 0).then_some(Self {
            strip,
            zone_count: capabilities.dial_count,
        })
    }

    /// Size of the whole strip.
    #[must_use]
    pub fn strip_size(&self) -> ImageSize {
        self.strip
    }

    /// Number of zones.
    #[must_use]
    pub fn zone_count(&self) -> u8 {
        self.zone_count
    }

    /// Size of one zone (the remainder of an uneven split is left unused).
    #[must_use]
    pub fn zone_size(&self) -> ImageSize {
        ImageSize {
            width: self.strip.width / u32::from(self.zone_count),
            height: self.strip.height,
        }
    }

    /// Left edge of `zone`.
    ///
    /// # Errors
    ///
    /// [`DeviceError::InvalidZone`] when the zone does not exist.
    pub fn zone_x(&self, zone: u8) -> Result<u32, DeviceError> {
        if zone < self.zone_count {
            Ok(u32::from(zone) * self.zone_size().width)
        } else {
            Err(DeviceError::InvalidZone {
                index: zone,
                count: self.zone_count,
            })
        }
    }

    /// Zone containing horizontal strip position `x`, if any.
    #[must_use]
    pub fn zone_at(&self, x: u16) -> Option<u8> {
        let zone = u32::from(x) / self.zone_size().width;
        u8::try_from(zone)
            .ok()
            .filter(|zone| *zone < self.zone_count)
    }

    /// Checks that `image` has the size of a zone.
    ///
    /// # Errors
    ///
    /// [`DeviceError::ImageSizeMismatch`] otherwise.
    pub fn check_zone_image(&self, image: &RgbaImage) -> Result<(), DeviceError> {
        check_image_size(image, self.zone_size())
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
    fn plus_strip_has_four_zones_of_200_by_100() {
        let layout = StripLayout::for_device(&info(4, Some(PLUS_STRIP))).unwrap();
        assert_eq!(layout.zone_count(), 4);
        assert_eq!(
            layout.zone_size(),
            ImageSize {
                width: 200,
                height: 100
            }
        );
        assert_eq!(layout.zone_x(3).unwrap(), 600);
        assert!(matches!(
            layout.zone_x(4),
            Err(DeviceError::InvalidZone { index: 4, count: 4 })
        ));
    }

    #[test]
    fn zone_at_maps_positions_to_zones() {
        let layout = StripLayout::for_device(&info(4, Some(PLUS_STRIP))).unwrap();
        assert_eq!(layout.zone_at(0), Some(0));
        assert_eq!(layout.zone_at(199), Some(0));
        assert_eq!(layout.zone_at(200), Some(1));
        assert_eq!(layout.zone_at(799), Some(3));
        assert_eq!(layout.zone_at(800), None);
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
