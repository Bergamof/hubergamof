//! Resolving the displayed page and composing its frame.

use super::{
    BTreeMap, ContextFace, DeviceEngine, DialAssignment, DisplayedPage, Frame, FrameKey,
    HIDE_NOTIFICATION, HOME_PAGE_ID, HOME_PAGE_TITLE, HeldKey, ImageSource, Instant, KeyFace,
    KeyGrid, KeyTarget, ModuleInstance, ModuleInstanceId, NotificationAction, NotificationFace,
    PageItem, PageModel, PageSummary, StripOverlay, UnreadNotification,
};

impl DeviceEngine {
    /// Displayed page; pages that no longer exist are popped.
    pub(super) fn resolve_page(&mut self) -> PageModel {
        loop {
            let Some(current) = self.nav.current().cloned() else {
                return PageModel::Home;
            };
            match self.instances.page(&current.module, &current.page) {
                Some(content) => {
                    self.nav.set_current_title(&content.title);
                    return PageModel::Module {
                        module: current.module,
                        page: current.page,
                        content,
                    };
                }
                None => self.nav.back(),
            }
        }
    }

    pub(super) fn displayed_page(&mut self) -> DisplayedPage {
        match self.resolve_page() {
            PageModel::Home => DisplayedPage {
                id: HOME_PAGE_ID.to_owned(),
                title: HOME_PAGE_TITLE.to_owned(),
                keys: self.home_keys(),
                grid: self.grid,
                context: None,
                keeps_lit: false,
            },
            PageModel::Module {
                module,
                page,
                content,
            } => {
                // What a page declares keeps the device lit only for a module
                // that declares it may (what a press does: `may_press`).
                let lock_safe = self.instances.lock_safe_pages(&module);
                let (items, grid) = self.placed_items(content.items, content.pinned_items);
                DisplayedPage {
                    id: format!("{module}/{page}"),
                    title: content.title,
                    keeps_lit: lock_safe && content.keeps_device_lit,
                    keys: items
                        .into_iter()
                        .map(|item| {
                            item.map(|item| FrameKey {
                                usable_when_locked: item.key.usable_when_locked,
                                face: KeyFace::from_content(item.key),
                                target: Some(KeyTarget::Item {
                                    module: module.clone(),
                                    page: page.clone(),
                                    item: item.id,
                                }),
                            })
                            .unwrap_or_default()
                        })
                        .collect(),
                    grid,
                    context: Some(ContextFace {
                        module,
                        page,
                        dial: content.context_dial,
                    }),
                }
            }
        }
    }

    /// The items of a module page as they are placed: the ones hidden on a
    /// locked computer left out, and the pinned ones (the first `pinned`
    /// declared) completed with blanks (`None`) to whole rows. Returns them
    /// with the grid that pins their rows.
    pub(super) fn placed_items(
        &self,
        items: Vec<PageItem>,
        pinned: usize,
    ) -> (Vec<Option<PageItem>>, KeyGrid) {
        let locked = self.lock.is_locked();
        let mut kept = Vec::with_capacity(items.len());
        let mut pinned_kept = 0;
        for (index, item) in items.into_iter().enumerate() {
            if locked && item.hidden_when_locked {
                continue;
            }
            pinned_kept += usize::from(index < pinned);
            kept.push(Some(item));
        }
        let grid = self.grid.pinning(pinned_kept);
        let blanks = grid.pinned_len().saturating_sub(pinned_kept);
        kept.splice(pinned_kept..pinned_kept, std::iter::repeat_n(None, blanks));
        (kept, grid)
    }

    /// Places `page` on the keys from the visible row, with the feedback
    /// marks, and adds the dials.
    pub(super) fn compose(&mut self, page: DisplayedPage) -> Frame {
        let grid = page.grid;
        let row = self.nav.row();
        let row_count = grid.row_count(page.keys.len());
        let keys = (0..self.deck.info().capabilities.key_count())
            .map(|key| {
                let shown = u8::try_from(key)
                    .ok()
                    .and_then(|key| page.keys.get(grid.item_index(key, row)));
                shown
                    .cloned()
                    .map(|key| self.marked(key))
                    .unwrap_or_default()
            })
            .collect();
        let positions = grid.max_row(page.keys.len()) + 1;
        let strip = self
            .has_strip
            .then(|| self.strip_face(&page.title, row, positions, page.context.as_ref()));
        Frame {
            strip,
            page: PageSummary {
                id: page.id,
                title: page.title,
                row,
                row_count,
            },
            keys,
            overlay: self.strip_overlay(),
            context: page.context,
            theme: self.config.theme,
            keeps_lit: page.keeps_lit,
        }
    }

    pub(super) fn marked(&self, mut key: FrameKey) -> FrameKey {
        key.face.locked = key
            .target
            .as_ref()
            .is_some_and(|target| !self.may_press(target.module(), key.usable_when_locked));
        key.face.feedback = key
            .target
            .as_ref()
            .and_then(|target| self.marks.key(target));
        key
    }

    /// Keys of the home modules, with the label and image of the
    /// configuration taking precedence over the module's.
    pub(super) fn home_keys(&mut self) -> Vec<FrameKey> {
        let configured: BTreeMap<&ModuleInstanceId, &ModuleInstance> = self
            .config
            .modules
            .iter()
            .map(|module| (&module.id, module))
            .collect();
        let mut keys = Vec::with_capacity(self.config.home.len());
        for id in &self.config.home {
            let content = self.instances.key(id);
            let usable_when_locked = content.as_ref().is_some_and(|key| key.usable_when_locked);
            let mut face = content.map(KeyFace::from_content).unwrap_or_default();
            if let Some(config) = configured.get(id) {
                if let Some(label) = &config.label {
                    face.label = Some(label.clone());
                }
                if let Some(name) = &config.image {
                    match self
                        .services
                        .store
                        .image_path(&self.deck.info().serial, name)
                    {
                        Ok(path) => face.image = Some(ImageSource::File(path)),
                        Err(error) => tracing::warn!(module = %id, %error, "ignoring image"),
                    }
                }
            }
            keys.push(FrameKey {
                face,
                target: Some(KeyTarget::Home(id.clone())),
                usable_when_locked,
            });
        }
        keys
    }

    /// Title of the page the navigation leads back to: the home page, or the
    /// module page that opened the displayed one, as it was when shown.
    pub(super) fn parent_title(&self) -> String {
        self.nav
            .parent()
            .map(|parent| parent.title.as_str())
            .filter(|title| !title.is_empty())
            .unwrap_or(HOME_PAGE_TITLE)
            .to_owned()
    }

    /// Scrolls to the key holding the cursor of the contextual dial when it
    /// moved, so that the user always sees it; a page the user scrolls
    /// elsewhere is left alone until the cursor moves again.
    pub(super) fn follow_focus(&mut self, keys: &[FrameKey], grid: KeyGrid) {
        let focused = keys.iter().position(|key| key.face.focused);
        let target = focused.and_then(|index| keys[index].target.clone());
        if target == self.focus {
            return;
        }
        if let Some(index) = focused {
            let row = grid.reveal(self.nav.row(), index, keys.len());
            self.nav.set_row(row);
        }
        self.focus = target;
    }

    /// Keeps the visible row within the page, which may have shrunk since
    /// the last scroll.
    pub(super) fn clamp_row(&mut self, item_count: usize, grid: KeyGrid) {
        let row = self.nav.row().min(grid.max_row(item_count));
        self.nav.set_row(row);
    }

    /// What covers the touch strip: the label of a held key, else the
    /// notification.
    pub(super) fn strip_overlay(&self) -> Option<StripOverlay> {
        if let Some(label) = self.held.as_ref().and_then(HeldKey::shown_label) {
            return Some(StripOverlay::HeldLabel {
                label: label.to_owned(),
                theme: self.config.theme,
            });
        }
        let shown = self.notifications.shown()?;
        Some(StripOverlay::Notification(NotificationFace::new(
            &shown.notification,
            shown.timeout,
            shown.elapsed(Instant::now()),
            self.notification_actions(&shown.notification),
        )))
    }

    /// The reminders of the dials that act under `notification`: the
    /// contextual dial if the notification declares a press, and the
    /// navigation dials, whose press hides it.
    pub(super) fn notification_actions(
        &self,
        notification: &UnreadNotification,
    ) -> Vec<NotificationAction> {
        let dials = self.config.dials.len();
        let contextual = notification.context_dial.hint.press.clone();
        self.config
            .dials
            .iter()
            .enumerate()
            .filter_map(|(dial, assignment)| {
                let text = match assignment {
                    DialAssignment::Contextual
                        if self.may_press(
                            &notification.source,
                            notification.context_dial.usable_when_locked,
                        ) =>
                    {
                        contextual.clone()?
                    }
                    DialAssignment::Navigation => HIDE_NOTIFICATION.to_owned(),
                    _ => return None,
                };
                Some(NotificationAction { text, dial, dials })
            })
            .collect()
    }
}
