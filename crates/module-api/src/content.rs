//! What a module shows on its keys and pages.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ItemId;

/// An encoded image supplied by a module: PNG, JPEG or SVG (uncompressed,
/// without external resources). The host recognises the format from the
/// content.
///
/// The host scales it to fit the target surface. The bytes are shared, so
/// returning the same `ImageData` on every call costs nothing; the host
/// compares contents to skip redrawing unchanged images.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageData {
    /// Encoded image.
    pub bytes: Arc<[u8]>,
}

impl ImageData {
    /// Wraps encoded bytes.
    #[must_use]
    pub fn new(bytes: impl Into<Arc<[u8]>>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }
}

/// Visual state of a key, drawn by the host on top of its content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[non_exhaustive]
pub enum KeyState {
    /// Nothing special.
    #[default]
    Normal,
    /// On, running, selected (toggle on, timer running…). The host draws
    /// nothing for it: the module shows it through its own icon.
    Active,
    /// Needs the user's attention (finished timer, failed container…).
    Attention,
}

/// Content of one key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyContent {
    /// Image, scaled to fit the key.
    pub image: Option<ImageData>,
    /// Text drawn at the bottom of the key.
    pub label: Option<String>,
    /// Visual state.
    pub state: KeyState,
}

impl KeyContent {
    /// A key showing only `label`.
    #[must_use]
    pub fn with_label(label: impl Into<String>) -> Self {
        Self {
            label: Some(label.into()),
            ..Self::default()
        }
    }

    /// A key showing `image` over `label`.
    #[must_use]
    pub fn with_icon(image: &ImageData, label: impl Into<String>) -> Self {
        Self {
            image: Some(image.clone()),
            label: Some(label.into()),
            state: KeyState::Normal,
        }
    }

    /// The same key in state `state`.
    #[must_use]
    pub fn in_state(self, state: KeyState) -> Self {
        Self { state, ..self }
    }
}

/// One key of a module page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageItem {
    /// Identifier passed back to [`crate::Module::on_item_press`].
    pub id: ItemId,
    /// What the key shows.
    pub key: KeyContent,
}

/// A page opened by a module with [`crate::Command::OpenPage`].
///
/// Items fill the keys row by row; when they do not fit, the host lets the
/// user scroll by rows ([`crate::Command::ScrollRows`]).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageContent {
    /// Title, shown in the UI and on the touch strip.
    pub title: String,
    /// Keys, in display order.
    pub items: Vec<PageItem>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_states_use_camel_case_on_the_wire() {
        assert_eq!(
            serde_json::to_value(KeyState::Attention).unwrap(),
            serde_json::json!("attention")
        );
        assert_eq!(KeyContent::default().state, KeyState::Normal);
        assert_eq!(
            KeyContent::with_label("x").in_state(KeyState::Active).state,
            KeyState::Active
        );
    }
}
