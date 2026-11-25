//! Key icon of the headphones module (SVG, drawn by the host).

use std::sync::LazyLock;

use hubergamof_module_api::ImageData;

use crate::icons::{asset, svg};

/// Headphones: the module.
pub(super) static HEADPHONES: LazyLock<ImageData> = LazyLock::new(|| svg(asset!("headphones")));
