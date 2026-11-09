//! Images the host stores for a device: their names, and how a module reads
//! the ones its settings refer to (the icons the user gave to its items).

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::ImageData;

/// A stored image being read; `None` when it is missing or unreadable.
pub type ImageFuture = Pin<Box<dyn Future<Output = Option<ImageData>> + Send>>;

/// Reads the stored images of the device a module instance belongs to, by
/// name (the names found in its settings). Reading is asynchronous: a module
/// loads from a background task, then keeps the result.
#[derive(Clone)]
pub struct ImageLibrary {
    load: Arc<dyn Fn(&str) -> ImageFuture + Send + Sync>,
}

impl ImageLibrary {
    /// A library reading images with `load`; built by the host.
    #[must_use]
    pub fn new(load: impl Fn(&str) -> ImageFuture + Send + Sync + 'static) -> Self {
        Self {
            load: Arc::new(load),
        }
    }

    /// A library holding no image.
    #[must_use]
    pub fn empty() -> Self {
        Self::new(|_| Box::pin(std::future::ready(None)))
    }

    /// Reads image `name`.
    pub fn load(&self, name: &str) -> ImageFuture {
        (self.load)(name)
    }
}

impl fmt::Debug for ImageLibrary {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImageLibrary")
            .finish_non_exhaustive()
    }
}

/// Longest accepted image file name.
pub const MAX_IMAGE_NAME_LENGTH: usize = 128;

/// Whether `name` is a plain file name of the device `images/` directory:
/// ASCII letters, digits, `-`, `_` and `.`, not starting with a dot (no
/// hidden file, no `..`), at most [`MAX_IMAGE_NAME_LENGTH`] characters. Such
/// a name cannot designate another directory.
#[must_use]
pub fn is_safe_image_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_IMAGE_NAME_LENGTH
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_file_names_are_safe() {
        for name in ["a.png", "0f3a-b_c.svg", "x"] {
            assert!(is_safe_image_name(name), "{name}");
        }
        for name in ["", ".hidden", "..", "a/b.png", "a b.png", "é.png"] {
            assert!(!is_safe_image_name(name), "{name}");
        }
        assert!(!is_safe_image_name(&"a".repeat(MAX_IMAGE_NAME_LENGTH + 1)));
    }
}
