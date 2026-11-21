//! What each module looks like in the menu: its name and its icon, the same as
//! on the deck and in the interface.

use std::collections::HashMap;

use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::{ImageSize, ModuleKind};
use image::RgbaImage;

use super::window::Petal;
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

    /// The petal of the entry of `module`.
    #[must_use]
    pub fn petal(&self, module: &ModuleKind) -> Petal {
        self.0.get(module).map_or_else(
            || Petal {
                label: title_case(&module.0),
                icon: None,
            },
            |look| Petal {
                label: look.name.clone(),
                icon: look.icon.clone(),
            },
        )
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
        let petal = Looks::default().petal(&"teams".into());
        assert_eq!(petal.label, "Teams");
        assert!(petal.icon.is_none());
    }

    #[test]
    fn every_builtin_module_is_named_and_most_have_an_icon() {
        let registry = crate::testing::builtin_registry();
        let looks = Looks::of_modules(&registry, &Renderer::new().unwrap());
        let pomodoro = looks.petal(&"pomodoro".into());
        assert_eq!(pomodoro.label, "Pomodoro");
        let icon = pomodoro.icon.expect("pomodoro has an icon");
        assert_eq!((icon.width(), icon.height()), (ICON_SIDE, ICON_SIDE));
        for factory in registry.factories() {
            let descriptor = factory.descriptor();
            assert_eq!(looks.petal(&descriptor.kind).label, descriptor.name);
        }
    }
}
