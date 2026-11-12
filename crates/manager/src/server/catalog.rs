//! Module catalog sent to the UI, built once at startup: the descriptor of
//! every registered module and its icon rendered like a key.

use std::collections::BTreeMap;
use std::sync::Arc;

use hubergamof_module_api::ModuleFactory;
use hubergamof_modules::ModuleRegistry;
use hubergamof_protocol::{CatalogModule, ModuleDescriptor, ModuleKind, SettingKind};

use super::UI_IMAGE_SIZE;
use crate::render::{ImageSource, RenderError, Renderer};

/// Catalog of every module of `registry`. A module whose icon cannot be
/// rendered is listed without one. CPU-bound: call it from a blocking
/// context.
#[must_use]
pub fn module_catalog(registry: &ModuleRegistry, renderer: &Renderer) -> Arc<[CatalogModule]> {
    registry
        .factories()
        .map(|factory| {
            let descriptor = factory.descriptor();
            let icon = factory
                .icon()
                .and_then(|icon| icon_data_url(renderer, &descriptor.kind, icon.bytes));
            let setting_icons = setting_icons(renderer, factory.as_ref(), &descriptor);
            CatalogModule {
                descriptor,
                icon,
                setting_icons,
                default_settings: factory.default_settings(),
            }
        })
        .collect()
}

/// The icons of the items of the list settings of `descriptor`, by setting key.
fn setting_icons(
    renderer: &Renderer,
    factory: &dyn ModuleFactory,
    descriptor: &ModuleDescriptor,
) -> BTreeMap<String, String> {
    descriptor
        .settings
        .iter()
        .filter(|field| matches!(field.kind, SettingKind::FileEntryList { .. }))
        .filter_map(|field| {
            let icon = factory.setting_item_icon(&field.key)?;
            let url = icon_data_url(renderer, &descriptor.kind, icon.bytes)?;
            Some((field.key.clone(), url))
        })
        .collect()
}

fn icon_data_url(renderer: &Renderer, kind: &ModuleKind, bytes: Arc<[u8]>) -> Option<String> {
    let to_data_url = || -> Result<String, RenderError> {
        let icon = renderer.render_image(UI_IMAGE_SIZE, &ImageSource::Bytes(bytes))?;
        icon.data_url()
    };
    to_data_url()
        .inspect_err(|error| tracing::warn!(kind = %kind.0, %error, "cannot render module icon"))
        .ok()
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use hubergamof_module_api::{
        ImageData, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError,
        ModuleFactory,
    };
    use serde_json::Value;

    use super::*;

    const PNG_DATA_URL_PREFIX: &str = "data:image/png;base64,";

    struct Iconic {
        kind: &'static str,
        icon: Option<&'static str>,
    }

    struct Inert;

    #[async_trait]
    impl Module for Inert {
        fn update_settings(&mut self, _: &Value) -> Result<(), ModuleError> {
            Ok(())
        }
    }

    impl ModuleFactory for Iconic {
        fn descriptor(&self) -> ModuleDescriptor {
            ModuleDescriptor {
                kind: self.kind.into(),
                name: self.kind.into(),
                description: String::new(),
                capabilities: ModuleCapabilities { key: true },
                dial_functions: Vec::new(),
                settings: Vec::new(),
            }
        }

        fn icon(&self) -> Option<ImageData> {
            self.icon.map(|source| ImageData::new(source.as_bytes()))
        }

        fn create(&self, _: ModuleContext, _: &Value) -> Result<Box<dyn Module>, ModuleError> {
            Ok(Box::new(Inert))
        }
    }

    fn catalog_of(factories: Vec<Iconic>) -> Arc<[CatalogModule]> {
        let mut registry = ModuleRegistry::default();
        for factory in factories {
            registry.register(Arc::new(factory));
        }
        module_catalog(&registry, &Renderer::new().unwrap())
    }

    #[test]
    fn compose_files_without_an_image_show_the_project_icon_not_the_module_one() {
        let registry = crate::testing::builtin_registry();
        let catalog = module_catalog(&registry, &Renderer::new().unwrap());
        let docker = catalog
            .iter()
            .find(|module| module.descriptor.kind.0 == "docker")
            .unwrap();
        let project_icon = docker.setting_icons.get("composeFiles").unwrap();
        assert!(project_icon.starts_with(PNG_DATA_URL_PREFIX));
        assert_ne!(Some(project_icon), docker.icon.as_ref());
        for module in catalog.iter().filter(|m| m.descriptor.kind.0 != "docker") {
            assert!(
                module.setting_icons.is_empty(),
                "{}",
                module.descriptor.kind.0
            );
        }
    }

    #[test]
    fn every_builtin_module_gets_a_rendered_icon_and_its_default_settings() {
        let registry = crate::testing::builtin_registry();
        let catalog = module_catalog(&registry, &Renderer::new().unwrap());
        assert_eq!(catalog.len(), registry.factories().count());
        for module in catalog.iter() {
            let icon = module.icon.as_deref().unwrap_or_default();
            let png = BASE64
                .decode(icon.strip_prefix(PNG_DATA_URL_PREFIX).unwrap_or_default())
                .unwrap();
            let decoded = image::load_from_memory(&png).unwrap();
            assert_eq!(
                (decoded.width(), decoded.height()),
                (UI_IMAGE_SIZE.width, UI_IMAGE_SIZE.height),
                "{}",
                module.descriptor.kind.0
            );
            let factory = registry.get(&module.descriptor.kind).unwrap();
            assert_eq!(module.default_settings, factory.default_settings());
        }
    }

    #[test]
    fn icons_are_rendered_as_png_and_missing_or_broken_ones_are_left_out() {
        let catalog = catalog_of(vec![
            Iconic {
                kind: "drawn",
                icon: Some(
                    r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 2"><rect width="1" height="1" fill="red"/></svg>"#,
                ),
            },
            Iconic {
                kind: "plain",
                icon: None,
            },
            Iconic {
                kind: "broken",
                icon: Some("not an image"),
            },
        ]);
        let icons: Vec<_> = catalog
            .iter()
            .map(|module| (module.descriptor.kind.0.as_str(), module.icon.as_deref()))
            .collect();
        assert_eq!(icons.len(), 3);
        assert!(matches!(icons[0], ("broken", None)), "{icons:?}");
        assert!(
            matches!(icons[1], ("drawn", Some(url)) if url.starts_with(PNG_DATA_URL_PREFIX)),
            "{icons:?}"
        );
        assert!(matches!(icons[2], ("plain", None)), "{icons:?}");
    }
}
