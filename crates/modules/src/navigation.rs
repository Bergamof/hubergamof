//! Navigation module: its dial scrolls the displayed page and goes back, its
//! zone shows where the user is.

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, ImageData, Module, ModuleCapabilities, ModuleContext, ModuleDescriptor, ModuleError,
    ModuleFactory, NavigationView, ZoneContent,
};
use serde_json::Value;

use crate::icons::{asset, svg};

/// Kind of the navigation module.
pub const NAVIGATION_KIND: &str = "navigation";

/// Icon of the module: scrolling arrows.
const ICON_SVG: &str = asset!("navigation");

/// Creates [`Navigation`] instances.
#[derive(Debug, Clone, Copy, Default)]
pub struct NavigationFactory;

impl ModuleFactory for NavigationFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            kind: NAVIGATION_KIND.into(),
            name: "Navigation".into(),
            description: "Molette : tourner fait défiler la page, appuyer revient en arrière."
                .into(),
            capabilities: ModuleCapabilities {
                key: false,
                dial: true,
                touch_zone: true,
            },
            settings: Vec::new(),
        }
    }

    fn icon(&self) -> Option<ImageData> {
        Some(svg(ICON_SVG))
    }

    fn create(
        &self,
        _ctx: ModuleContext,
        _settings: &Value,
    ) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(Navigation))
    }
}

/// Scrolls rows on rotation, goes back on press.
#[derive(Debug)]
pub struct Navigation;

#[async_trait]
impl Module for Navigation {
    async fn on_dial_rotate(&mut self, ticks: i32) -> Vec<Command> {
        vec![Command::ScrollRows(ticks)]
    }

    async fn on_dial_press(&mut self) -> Vec<Command> {
        vec![Command::Back]
    }

    fn zone(&self, nav: &NavigationView) -> Option<ZoneContent> {
        let mut lines = vec![nav.page_title.clone()];
        if !nav.is_home() || nav.is_scrollable() {
            lines.push(format!("ligne {}/{}", nav.row + 1, nav.row_count.max(1)));
        }
        Some(ZoneContent::Lines(lines))
    }

    fn update_settings(&mut self, _settings: &Value) -> Result<(), ModuleError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(title: &str, depth: usize, row: u32, row_count: u32) -> NavigationView {
        let view = NavigationView::new(title, row, row_count, 2);
        if depth == 0 {
            view
        } else {
            view.on_page("docker".into(), "page".into(), depth)
        }
    }

    fn create() -> Box<dyn Module> {
        NavigationFactory
            .create(ModuleContext::for_tests("nav"), &Value::Null)
            .unwrap()
    }

    #[tokio::test]
    async fn rotation_scrolls_and_press_goes_back() {
        let mut nav = create();
        assert_eq!(nav.on_dial_rotate(-2).await, [Command::ScrollRows(-2)]);
        assert_eq!(nav.on_dial_press().await, [Command::Back]);
        assert_eq!(nav.on_key_press().await, []);
    }

    #[test]
    fn zone_shows_home_or_page_and_row() {
        let nav = create();
        assert_eq!(
            nav.zone(&view("Accueil", 0, 0, 1)),
            Some(ZoneContent::Lines(vec!["Accueil".into()]))
        );
        assert_eq!(
            nav.zone(&view("Accueil", 0, 0, 2)),
            Some(ZoneContent::Lines(vec!["Accueil".into()]))
        );
        assert_eq!(
            nav.zone(&view("Accueil", 0, 1, 3)),
            Some(ZoneContent::Lines(vec![
                "Accueil".into(),
                "ligne 2/3".into()
            ]))
        );
        assert_eq!(
            nav.zone(&view("Conteneurs", 1, 0, 1)),
            Some(ZoneContent::Lines(vec![
                "Conteneurs".into(),
                "ligne 1/1".into()
            ]))
        );
    }

    #[test]
    fn has_no_settings() {
        assert_eq!(
            NavigationFactory.default_settings(),
            Value::Object(serde_json::Map::new())
        );
        assert!(create().update_settings(&Value::Null).is_ok());
    }
}
