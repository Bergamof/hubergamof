//! What each module looks like in the menu: its name and its icon, the same as
//! on the deck and in the interface.

use std::collections::HashMap;
use std::sync::Arc;

use hubergamof_module_api::{MAX_RADIAL_ITEMS, RadialItem};
use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::{ImageSize, ModuleKind};
use image::RgbaImage;

use super::window::{Petal, PetalNote};
use crate::render::{ImageSource, Renderer};

/// Side, in pixels, the icons are rendered at: more than the 36 the menu draws
/// them at, so that a high-density screen stays sharp.
const ICON_SIDE: u32 = 96;

#[derive(Debug, Clone)]
struct Look {
    name: String,
    icon: Option<RgbaImage>,
}

/// The name and the icon of each module. A module it knows nothing about shows
/// its identifier, without an icon.
#[derive(Debug, Clone, Default)]
pub struct Looks(HashMap<ModuleKind, Look>);

impl Looks {
    /// The looks of every module of `registry`. A module whose icon cannot be
    /// rendered is named, without an icon. CPU-bound: call it from a blocking
    /// context.
    #[must_use]
    pub fn of_modules(registry: &ModuleRegistry, renderer: &Renderer) -> Self {
        let size = ImageSize {
            width: ICON_SIDE,
            height: ICON_SIDE,
        };
        let looks = registry
            .factories()
            .map(|factory| {
                let descriptor = factory.descriptor();
                let icon = factory.icon().and_then(|icon| {
                    renderer
                        .render_image(size, &ImageSource::Bytes(icon.bytes))
                        .inspect_err(|error| {
                            tracing::warn!(kind = %descriptor.kind.0, %error, "no icon for the radial menu");
                        })
                        .ok()
                        .map(|rendered| RgbaImage::clone(rendered.image()))
                });
                (
                    descriptor.kind,
                    Look {
                        name: descriptor.name,
                        icon,
                    },
                )
            })
            .collect();
        Self(looks)
    }

    /// The petal of the entry of `module`, with what it says now.
    #[must_use]
    pub fn petal(&self, module: &ModuleKind, note: PetalNote) -> Petal {
        match self.0.get(module) {
            Some(look) => Petal {
                label: look.name.clone(),
                icon: look.icon.clone(),
                note,
                items: Vec::new(),
            },
            None => Petal {
                label: title_case(&module.0),
                icon: None,
                note,
                items: Vec::new(),
            },
        }
    }
}

/// What each module says on its petal in the menu (a status under its name, a
/// badge on its icon), read when the menu opens and again while it stays open: what the modules show now, where `Looks` has
/// what they always look like.
#[derive(Clone, Default)]
pub struct PetalNotes(Option<NoteReader>);

/// Reads what a module says now.
type NoteReader = Arc<dyn Fn(&ModuleKind) -> PetalNote + Send + Sync>;

impl PetalNotes {
    /// The notes read by `read`.
    #[must_use]
    pub fn new(read: impl Fn(&ModuleKind) -> PetalNote + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(read)))
    }

    /// What `module` says now, if it says anything.
    #[must_use]
    pub fn of(&self, module: &ModuleKind) -> PetalNote {
        self.0.as_ref().map(|read| read(module)).unwrap_or_default()
    }
}

impl std::fmt::Debug for PetalNotes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PetalNotes")
            .field(&self.0.is_some())
            .finish()
    }
}

/// The actions each module offers around its petal now (the controls of a
/// call during a call), read when the menu opens and again while it stays
/// open: they come and go, unlike what [`Looks`] knows.
#[derive(Clone, Default)]
pub struct Items(Option<ItemReader>);

/// Reads the actions a module offers now.
type ItemReader = Arc<dyn Fn(&ModuleKind) -> Vec<RadialItem> + Send + Sync>;

impl Items {
    /// The items read by `read`.
    #[must_use]
    pub fn new(read: impl Fn(&ModuleKind) -> Vec<RadialItem> + Send + Sync + 'static) -> Self {
        Self(Some(Arc::new(read)))
    }

    /// What `module` offers now, at most [`MAX_RADIAL_ITEMS`]: the arc has no
    /// room for more.
    #[must_use]
    pub fn of(&self, module: &ModuleKind) -> Vec<RadialItem> {
        let mut items = self.0.as_ref().map(|read| read(module)).unwrap_or_default();
        items.truncate(MAX_RADIAL_ITEMS);
        items
    }
}

impl std::fmt::Debug for Items {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Items").field(&self.0.is_some()).finish()
    }
}

/// `teams` as `Teams`.
fn title_case(name: &str) -> String {
    let mut characters = name.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_module_nobody_described_shows_its_identifier() {
        let petal = Looks::default().petal(&"teams".into(), PetalNote::default());
        assert_eq!(petal.label, "Teams");
        assert!(petal.icon.is_none());
    }

    #[test]
    fn every_builtin_module_is_named_and_most_have_an_icon() {
        let registry = crate::testing::builtin_registry();
        let looks = Looks::of_modules(&registry, &Renderer::new().unwrap());
        let pomodoro = looks.petal(
            &"pomodoro".into(),
            PetalNote {
                status: Some("Focus".to_owned()),
                badge: Some(3),
            },
        );
        assert_eq!(pomodoro.note.status.as_deref(), Some("Focus"));
        assert_eq!(pomodoro.note.badge, Some(3));
        assert_eq!(pomodoro.label, "Pomodoro");
        let icon = pomodoro.icon.expect("pomodoro has an icon");
        assert_eq!((icon.width(), icon.height()), (ICON_SIDE, ICON_SIDE));
        for factory in registry.factories() {
            let descriptor = factory.descriptor();
            assert_eq!(
                looks.petal(&descriptor.kind, PetalNote::default()).label,
                descriptor.name
            );
        }
    }
}

#[cfg(test)]
mod note_tests {
    use super::*;

    #[test]
    fn notes_say_what_the_reader_says_and_nothing_by_default() {
        assert_eq!(
            PetalNotes::default().of(&"pomodoro".into()),
            PetalNote::default()
        );
        let notes = PetalNotes::new(|kind| PetalNote {
            status: (kind.0 == "pomodoro").then(|| "Focus".to_owned()),
            badge: (kind.0 == "teams").then_some(4),
        });
        assert_eq!(
            notes.of(&"pomodoro".into()).status.as_deref(),
            Some("Focus")
        );
        assert_eq!(notes.of(&"teams".into()).badge, Some(4));
        assert_eq!(notes.of(&"audio".into()), PetalNote::default());
    }
}

#[cfg(test)]
mod item_tests {
    use hubergamof_module_api::RadialGlyph;

    use super::*;

    fn item(id: &str) -> RadialItem {
        RadialItem::new(id, id, RadialGlyph::Play)
    }

    #[test]
    fn items_are_what_the_reader_says_and_none_by_default() {
        assert_eq!(Items::default().of(&"teams".into()), Vec::new());
        let items = Items::new(|kind| {
            if kind.0 == "teams" {
                vec![item("mute")]
            } else {
                Vec::new()
            }
        });
        assert_eq!(items.of(&"teams".into()), [item("mute")]);
        assert_eq!(items.of(&"audio".into()), Vec::new());
    }

    #[test]
    fn a_module_that_offers_too_many_is_cut_at_the_room_of_the_arc() {
        let items = Items::new(|_| (0..10).map(|n| item(&n.to_string())).collect());
        let read = items.of(&"teams".into());
        assert_eq!(read.len(), MAX_RADIAL_ITEMS);
        assert_eq!(read[0].id, "0", "the first ones are kept");
    }
}
