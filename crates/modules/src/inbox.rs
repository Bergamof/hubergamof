//! Notifications module: a bell with the number of unread notifications,
//! and the list of them. Choosing one shows it again over the touch strip,
//! with no timeout.

use std::sync::LazyLock;

use async_trait::async_trait;
use hubergamof_module_api::{
    Command, ImageData, ItemId, KeyContent, Module, ModuleCapabilities, ModuleContext,
    ModuleDescriptor, ModuleError, ModuleFactory, NotificationId, PageContent, PageId, PageItem,
    UnreadNotification, UnreadNotifications,
};
use serde_json::Value;

use crate::icons::{asset, svg};

/// Kind of the notifications module.
pub const INBOX_KIND: &str = "notifications";

/// Icon of the module: a bell.
const ICON_SVG: &str = asset!("notifications");

/// The only page: the unread notifications.
const UNREAD_PAGE: &str = "unread";

/// Item of the page standing for "nothing to show".
const EMPTY_ITEM: &str = "none";

/// Prefix of the id of an item standing for a notification.
const ITEM_PREFIX: &str = "n";

/// Name of the module: label of its key and title of its page.
const NAME: &str = "Notifications";

/// Label of the placeholder item of an empty list.
const EMPTY_LABEL: &str = "Aucune notification";

/// Most unread notifications the badge counts exactly.
const BADGE_MAX: usize = 99;

/// Where a badge is inserted in the bell.
const SVG_END: &str = "</svg>";

/// The bell with the badge of every count, `0` being the bell alone and
/// `BADGE_MAX + 1` the badge of more than [`BADGE_MAX`]: built once, as the
/// host remembers images by their allocation.
static BELLS: LazyLock<Vec<ImageData>> =
    LazyLock::new(|| (0..=BADGE_MAX + 1).map(draw_bell).collect());

/// The bell, as shown on the key of a module without a notification of its
/// own.
static BELL: LazyLock<ImageData> = LazyLock::new(|| svg(ICON_SVG));

/// Creates [`Inbox`] instances.
#[derive(Debug, Clone, Copy, Default)]
pub struct InboxFactory;

impl ModuleFactory for InboxFactory {
    fn descriptor(&self) -> ModuleDescriptor {
        ModuleDescriptor {
            kind: INBOX_KIND.into(),
            name: NAME.into(),
            description: "Cloche avec le nombre de notifications non lues ; appuyer les liste, \
                          en choisir une l'affiche de nouveau sur la bande tactile."
                .into(),
            capabilities: ModuleCapabilities {
                key: true,
                dial: false,
                touch_zone: false,
            },
            settings: Vec::new(),
        }
    }

    fn icon(&self) -> Option<ImageData> {
        Some(BELL.clone())
    }

    fn create(
        &self,
        ctx: ModuleContext,
        _settings: &Value,
    ) -> Result<Box<dyn Module>, ModuleError> {
        Ok(Box::new(Inbox {
            unread: ctx.unread_notifications().clone(),
        }))
    }
}

/// Counts and lists the unread notifications.
#[derive(Debug)]
pub struct Inbox {
    unread: UnreadNotifications,
}

#[async_trait]
impl Module for Inbox {
    fn key(&self) -> KeyContent {
        KeyContent::with_icon(&bell_with_badge(self.unread.count()), NAME)
    }

    async fn on_key_press(&mut self) -> Vec<Command> {
        vec![Command::OpenPage(UNREAD_PAGE.into())]
    }

    fn page(&self, page: &PageId) -> Option<PageContent> {
        (page.0 == UNREAD_PAGE).then(|| {
            let mut items: Vec<PageItem> = self
                .unread
                .list()
                .iter()
                // Newest first.
                .rev()
                .map(item_of)
                .collect();
            if items.is_empty() {
                items.push(PageItem {
                    id: ItemId(EMPTY_ITEM.into()),
                    key: KeyContent::with_icon(&BELL, EMPTY_LABEL),
                });
            }
            PageContent {
                title: NAME.into(),
                items,
            }
        })
    }

    async fn on_item_press(&mut self, _page: &PageId, item: &ItemId) -> Vec<Command> {
        match notification_id(item) {
            Some(id) => vec![Command::ShowNotification(id)],
            None => vec![Command::Back],
        }
    }

    fn update_settings(&mut self, _settings: &Value) -> Result<(), ModuleError> {
        Ok(())
    }
}

/// The key of `notification` on the list: its sender's icon over its message.
fn item_of(notification: &UnreadNotification) -> PageItem {
    let icon = notification.icon.as_ref().unwrap_or(&BELL);
    PageItem {
        id: ItemId(format!("{ITEM_PREFIX}{}", notification.id.0)),
        key: KeyContent::with_icon(icon, notification.message.clone()),
    }
}

fn notification_id(item: &ItemId) -> Option<NotificationId> {
    item.0
        .strip_prefix(ITEM_PREFIX)?
        .parse()
        .ok()
        .map(NotificationId)
}

/// The bell, with a red badge holding `count` at its top right corner when
/// there are unread notifications.
fn bell_with_badge(count: usize) -> ImageData {
    BELLS[count.min(BADGE_MAX + 1)].clone()
}

fn draw_bell(count: usize) -> ImageData {
    if count == 0 {
        return BELL.clone();
    }
    let (text, font_size) = if count > BADGE_MAX {
        (format!("{BADGE_MAX}+"), 17)
    } else if count > 9 {
        (count.to_string(), 20)
    } else {
        (count.to_string(), 24)
    };
    let badge = format!(
        r##"<circle cx="98" cy="30" r="24" fill="#000"/><circle cx="98" cy="30" r="19" fill="#F0362B"/><text x="98" y="{}" text-anchor="middle" font-family="Noto Sans" font-weight="600" font-size="{font_size}" fill="#fff">{text}</text>"##,
        30 + font_size / 3
    );
    let bell = ICON_SVG.trim_end();
    let body = bell.strip_suffix(SVG_END).unwrap_or(bell);
    svg(&format!("{body}{badge}{SVG_END}"))
}

#[cfg(test)]
mod tests {
    use hubergamof_module_api::{Notification, Rgb};

    use super::*;

    fn inbox_with(messages: &[&str]) -> Inbox {
        let list: Vec<_> = messages
            .iter()
            .zip(0..)
            .map(|(message, id)| {
                UnreadNotification::new(
                    NotificationId(id),
                    "pomodoro".into(),
                    None,
                    &Notification::new(*message, Rgb(1, 2, 3)),
                )
            })
            .collect();
        Inbox {
            unread: UnreadNotifications::new(move || list.clone()),
        }
    }

    fn icon_source(content: &KeyContent) -> String {
        String::from_utf8(content.image.clone().unwrap().bytes.to_vec()).unwrap()
    }

    #[test]
    fn the_bell_alone_when_everything_is_read() {
        let key = inbox_with(&[]).key();
        assert_eq!(icon_source(&key), ICON_SVG);
        assert_eq!(key.label.as_deref(), Some(NAME));
    }

    #[test]
    fn the_badge_shows_the_count_up_to_99() {
        for (count, shown) in [(1, ">1<"), (12, ">12<"), (99, ">99<"), (100, ">99+<")] {
            let messages = vec!["x"; count];
            let source = icon_source(&inbox_with(&messages).key());
            assert!(source.contains(shown), "{count}: {source}");
            assert!(crate::icons::is_complete_svg(&source), "{count}");
        }
    }

    #[tokio::test]
    async fn pressing_the_key_opens_the_list_newest_first() {
        let mut inbox = inbox_with(&["ancienne", "récente"]);
        assert_eq!(
            inbox.on_key_press().await,
            [Command::OpenPage(UNREAD_PAGE.into())]
        );
        let page = inbox.page(&UNREAD_PAGE.into()).unwrap();
        let labels: Vec<_> = page
            .items
            .iter()
            .map(|item| item.key.label.clone().unwrap())
            .collect();
        assert_eq!(labels, ["récente", "ancienne"]);
        assert!(inbox.page(&"other".into()).is_none());
    }

    #[tokio::test]
    async fn choosing_a_notification_shows_it_again() {
        let mut inbox = inbox_with(&["a", "b"]);
        let page = inbox.page(&UNREAD_PAGE.into()).unwrap();
        let newest = &page.items[0].id;
        assert_eq!(
            inbox.on_item_press(&UNREAD_PAGE.into(), newest).await,
            [Command::ShowNotification(NotificationId(1))]
        );
    }

    #[tokio::test]
    async fn an_empty_list_says_so_and_goes_back() {
        let mut inbox = inbox_with(&[]);
        let page = inbox.page(&UNREAD_PAGE.into()).unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].key.label.as_deref(), Some(EMPTY_LABEL));
        assert_eq!(
            inbox
                .on_item_press(&UNREAD_PAGE.into(), &page.items[0].id)
                .await,
            [Command::Back]
        );
    }

    #[test]
    fn the_descriptor_declares_a_key_only_module_without_settings() {
        let descriptor = InboxFactory.descriptor();
        assert_eq!(descriptor.kind.0, INBOX_KIND);
        assert!(descriptor.capabilities.key);
        assert!(!descriptor.capabilities.dial && !descriptor.capabilities.touch_zone);
        assert_eq!(descriptor.settings, []);
    }
}
