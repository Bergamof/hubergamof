//! Tray icon image: the HuBergamof mark, in the sizes hosts ask for.
//!
//! A raster icon travels with the item (`IconPixmap`) instead of a themed name: no icon theme
//! has the mark, and the tile is self-contained, so it reads on light and dark panels alike.

use std::sync::LazyLock;

use super::item::Pixmap;

/// The mark rendered from `assets/tray/hubergamof.svg`, smallest first.
const ICON_PNGS: [&[u8]; 4] = [
    include_bytes!("../../assets/tray/hubergamof-22.png"),
    include_bytes!("../../assets/tray/hubergamof-32.png"),
    include_bytes!("../../assets/tray/hubergamof-48.png"),
    include_bytes!("../../assets/tray/hubergamof-64.png"),
];

static PIXMAPS: LazyLock<Vec<Pixmap>> = LazyLock::new(|| {
    ICON_PNGS
        .iter()
        .filter_map(|png| match decode(png) {
            Ok(pixmap) => Some(pixmap),
            Err(error) => {
                tracing::warn!(%error, "cannot decode an embedded tray icon");
                None
            }
        })
        .collect()
});

/// The icon in every embedded size.
pub fn pixmaps() -> Vec<Pixmap> {
    PIXMAPS.clone()
}

/// PNG to the ARGB32 (network byte order) image of the specification.
fn decode(png: &[u8]) -> Result<Pixmap, image::ImageError> {
    let rgba = image::load_from_memory_with_format(png, image::ImageFormat::Png)?.into_rgba8();
    let (width, height) = rgba.dimensions();
    let data = rgba
        .pixels()
        .flat_map(|pixel| {
            let [red, green, blue, alpha] = pixel.0;
            [alpha, red, green, blue]
        })
        .collect();
    Ok(Pixmap::new(width, height, data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_embedded_size_decodes_to_argb() {
        let sizes: Vec<_> = pixmaps().iter().map(Pixmap::size).collect();
        assert_eq!(sizes, [22, 32, 48, 64]);
        for pixmap in pixmaps() {
            let side = pixmap.size();
            assert_eq!(pixmap.data().len(), side * side * 4);
        }
    }

    #[test]
    fn the_tile_is_rounded_and_opaque_inside() {
        let pixmaps = pixmaps();
        let largest = pixmaps.last().expect("embedded icons");
        let side = largest.size();
        let alpha_at = |x: usize, y: usize| largest.data()[(y * side + x) * 4];
        assert_eq!(alpha_at(0, 0), 0, "corner outside the rounded tile");
        assert_eq!(alpha_at(side / 2, side / 2), 255, "centre of the tile");
    }
}
