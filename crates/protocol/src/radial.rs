//! The radial menu: one configuration for every mouse, persisted as
//! `config/radial-menu.json`.

use std::collections::HashSet;
use std::ops::RangeInclusive;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use ts_rs::TS;

use crate::{ModuleKind, is_identifier};

/// Radial menu schema version produced and understood by this crate.
pub const RADIAL_SCHEMA_VERSION: u32 = 1;

/// Time, in milliseconds, a button must be held for the menu to follow a
/// press-and-release instead of staying open.
pub const HOLD_RANGE: RangeInclusive<u16> = 150..=500;

/// Hold time of a menu that does not say, in milliseconds.
pub const DEFAULT_HOLD_MS: u16 = 280;

/// Fewest entries shown: under this the petals get too wide to read as a menu.
pub const MIN_SHOWN_ENTRIES: usize = 3;

/// Most entries shown: past this the petals get too narrow to hit.
pub const MAX_SHOWN_ENTRIES: usize = 8;

/// Most entries of the list (shown or not): one per module is plenty.
pub const MAX_LISTED_ENTRIES: usize = 32;

/// What a click on a petal does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum RadialPrimary {
    /// Nothing: the menu just closes.
    Close,
    /// An action of the module.
    Action {
        /// The action, one the module offers in its radial entry.
        action: String,
    },
}

/// A module's place in the radial menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RadialEntryConfig {
    /// The module that provides the entry.
    pub module: ModuleKind,
    /// Shown in the menu.
    pub enabled: bool,
    /// What a click on the petal does; the module's other actions open around
    /// the petal on hover.
    pub primary: RadialPrimary,
}

/// The radial menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RadialMenuConfig {
    /// Schema version, see [`RADIAL_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// How long, in milliseconds, a press must last to count as a hold, within
    /// [`HOLD_RANGE`].
    pub hold_ms: u16,
    /// Open the petals with an animation.
    pub animate: bool,
    /// Play haptic feedback when the menu opens, on hover and on selection
    /// (mice that can).
    pub haptics: bool,
    /// Entries, in clockwise order from the top; only the enabled ones are
    /// shown.
    pub entries: Vec<RadialEntryConfig>,
    /// Revision counter, incremented on every accepted update; an update
    /// carrying an outdated revision is rejected.
    pub revision: u64,
}

/// Why a radial menu configuration is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RadialConfigError {
    /// The schema version is not supported by this version of HuBergamof.
    #[error("unsupported radial menu schema version {found} (supported: {RADIAL_SCHEMA_VERSION})")]
    UnsupportedSchemaVersion {
        /// Version found in the configuration.
        found: u32,
    },
    /// A hold time outside [`HOLD_RANGE`].
    #[error("hold time of {0} ms is outside {HOLD_RANGE:?}")]
    InvalidHold(u16),
    /// Too many entries in the list.
    #[error("{count} entries (at most {MAX_LISTED_ENTRIES})")]
    TooManyEntries {
        /// Number of entries found.
        count: usize,
    },
    /// A module listed twice.
    #[error("module `{0}` is listed twice")]
    DuplicateModule(ModuleKind),
    /// A module or action identifier that is not a plain identifier.
    #[error("invalid identifier {0:?}")]
    InvalidIdentifier(String),
    /// Fewer than [`MIN_SHOWN_ENTRIES`] or more than [`MAX_SHOWN_ENTRIES`]
    /// entries shown.
    #[error("{shown} entries shown, between {MIN_SHOWN_ENTRIES} and {MAX_SHOWN_ENTRIES} wanted")]
    ShownEntries {
        /// Number of enabled entries.
        shown: usize,
    },
}

impl RadialMenuConfig {
    /// The menu of a first start: Pomodoro, Teams, Audio and `IntelliJ` shown,
    /// the other modules listed and hidden.
    ///
    /// Provisional: the module and action ids are written here by hand until
    /// the modules declare their radial entries (module API, step 4), from
    /// which the host will build this menu; a test in the modules crate will
    /// then check that each exists.
    #[must_use]
    pub fn default_menu() -> Self {
        let entry = |module: &str, action: &str, enabled: bool| RadialEntryConfig {
            module: module.into(),
            enabled,
            primary: RadialPrimary::Action {
                action: action.to_owned(),
            },
        };
        Self {
            schema_version: RADIAL_SCHEMA_VERSION,
            hold_ms: DEFAULT_HOLD_MS,
            animate: true,
            haptics: true,
            entries: vec![
                entry("pomodoro", "toggle", true),
                entry("teams", "mic", true),
                entry("audio", "mute", true),
                entry("intellij", "run", true),
                entry("docker", "start", false),
                entry("claude", "accept", false),
                entry("notifications", "read", false),
            ],
            revision: 0,
        }
    }

    /// The entries shown, in order.
    pub fn shown(&self) -> impl Iterator<Item = &RadialEntryConfig> {
        self.entries.iter().filter(|entry| entry.enabled)
    }

    /// Checks the configuration.
    ///
    /// # Errors
    ///
    /// Returns the first [`RadialConfigError`] found.
    pub fn validate(&self) -> Result<(), RadialConfigError> {
        if self.schema_version != RADIAL_SCHEMA_VERSION {
            return Err(RadialConfigError::UnsupportedSchemaVersion {
                found: self.schema_version,
            });
        }
        if !HOLD_RANGE.contains(&self.hold_ms) {
            return Err(RadialConfigError::InvalidHold(self.hold_ms));
        }
        if self.entries.len() > MAX_LISTED_ENTRIES {
            return Err(RadialConfigError::TooManyEntries {
                count: self.entries.len(),
            });
        }
        let mut modules = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            if !is_identifier(&entry.module.0) {
                return Err(RadialConfigError::InvalidIdentifier(entry.module.0.clone()));
            }
            if let RadialPrimary::Action { action } = &entry.primary
                && !is_identifier(action)
            {
                return Err(RadialConfigError::InvalidIdentifier(action.clone()));
            }
            if !modules.insert(&entry.module) {
                return Err(RadialConfigError::DuplicateModule(entry.module.clone()));
            }
        }
        let shown = self.shown().count();
        if (MIN_SHOWN_ENTRIES..=MAX_SHOWN_ENTRIES).contains(&shown) {
            Ok(())
        } else {
            Err(RadialConfigError::ShownEntries { shown })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_menu_is_valid_and_shows_the_four_modules_asked_for() {
        let menu = RadialMenuConfig::default_menu();
        assert_eq!(menu.validate(), Ok(()));
        let shown: Vec<&str> = menu.shown().map(|entry| entry.module.0.as_str()).collect();
        assert_eq!(shown, ["pomodoro", "teams", "audio", "intellij"]);
        assert_eq!(menu.entries.len(), 7);
    }

    #[test]
    fn the_default_hold_time_is_in_range() {
        assert!(HOLD_RANGE.contains(&DEFAULT_HOLD_MS));
    }

    #[test]
    fn the_hold_time_stays_in_range() {
        let mut menu = RadialMenuConfig::default_menu();
        for (hold, valid) in [
            (0, false),
            (149, false),
            (150, true),
            (500, true),
            (501, false),
            (u16::MAX, false),
        ] {
            menu.hold_ms = hold;
            let expected = if valid {
                Ok(())
            } else {
                Err(RadialConfigError::InvalidHold(hold))
            };
            assert_eq!(menu.validate(), expected, "{hold} ms");
        }
    }

    fn menu_showing(shown: usize, listed: usize) -> RadialMenuConfig {
        let mut menu = RadialMenuConfig::default_menu();
        menu.entries = (0..listed)
            .map(|index| RadialEntryConfig {
                module: format!("m{index}").as_str().into(),
                enabled: index < shown,
                primary: RadialPrimary::Close,
            })
            .collect();
        menu
    }

    #[test]
    fn three_to_eight_entries_are_shown() {
        for (shown, valid) in [
            (0, false),
            (2, false),
            (3, true),
            (4, true),
            (7, true),
            (8, true),
            (9, false),
        ] {
            let expected = if valid {
                Ok(())
            } else {
                Err(RadialConfigError::ShownEntries { shown })
            };
            assert_eq!(
                menu_showing(shown, 12).validate(),
                expected,
                "{shown} shown"
            );
        }
    }

    #[test]
    fn the_list_may_be_long_when_few_are_shown() {
        assert_eq!(
            menu_showing(4, MAX_LISTED_ENTRIES).validate(),
            Ok(()),
            "exactly the limit"
        );
        assert_eq!(
            menu_showing(4, MAX_LISTED_ENTRIES + 1).validate(),
            Err(RadialConfigError::TooManyEntries {
                count: MAX_LISTED_ENTRIES + 1
            })
        );
    }

    #[test]
    fn a_module_appears_once_shown_or_not() {
        let mut menu = RadialMenuConfig::default_menu();
        let mut hidden_twin = menu.entries[0].clone();
        hidden_twin.enabled = false;
        menu.entries.push(hidden_twin);
        assert_eq!(
            menu.validate(),
            Err(RadialConfigError::DuplicateModule("pomodoro".into()))
        );
    }

    #[test]
    fn identifiers_are_plain() {
        for bad in ["po mo", "", "a;b", "é"] {
            let mut menu = RadialMenuConfig::default_menu();
            menu.entries[0].module = bad.into();
            assert_eq!(
                menu.validate(),
                Err(RadialConfigError::InvalidIdentifier(bad.to_owned())),
                "module {bad:?}"
            );
            let mut menu = RadialMenuConfig::default_menu();
            menu.entries[1].primary = RadialPrimary::Action {
                action: bad.to_owned(),
            };
            assert_eq!(
                menu.validate(),
                Err(RadialConfigError::InvalidIdentifier(bad.to_owned())),
                "action {bad:?}"
            );
        }
    }

    #[test]
    fn the_first_problem_found_is_the_one_reported() {
        // Version, then hold time, then the size of the list, then each entry.
        let mut menu = menu_showing(4, MAX_LISTED_ENTRIES + 1);
        menu.hold_ms = 0;
        menu.schema_version = 9;
        assert_eq!(
            menu.validate(),
            Err(RadialConfigError::UnsupportedSchemaVersion { found: 9 })
        );
        menu.schema_version = RADIAL_SCHEMA_VERSION;
        assert_eq!(menu.validate(), Err(RadialConfigError::InvalidHold(0)));
        menu.hold_ms = DEFAULT_HOLD_MS;
        assert_eq!(
            menu.validate(),
            Err(RadialConfigError::TooManyEntries {
                count: MAX_LISTED_ENTRIES + 1
            })
        );
    }

    #[test]
    fn a_click_that_only_closes_is_allowed() {
        let mut menu = RadialMenuConfig::default_menu();
        menu.entries[0].primary = RadialPrimary::Close;
        assert_eq!(menu.validate(), Ok(()));
    }

    #[test]
    fn another_schema_version_is_refused() {
        let mut menu = RadialMenuConfig::default_menu();
        menu.schema_version = 0;
        assert_eq!(
            menu.validate(),
            Err(RadialConfigError::UnsupportedSchemaVersion { found: 0 })
        );
    }

    #[test]
    fn the_menu_survives_json_and_keeps_its_shape() {
        let mut menu = RadialMenuConfig::default_menu();
        menu.entries[1].primary = RadialPrimary::Close;
        let json = serde_json::to_value(&menu).unwrap();
        assert_eq!(json["holdMs"], DEFAULT_HOLD_MS);
        assert_eq!(json["entries"][0]["module"], "pomodoro");
        assert_eq!(json["entries"][0]["primary"]["kind"], "action");
        assert_eq!(json["entries"][0]["primary"]["action"], "toggle");
        assert_eq!(json["entries"][1]["primary"]["kind"], "close");
        let back: RadialMenuConfig = serde_json::from_value(json).unwrap();
        assert_eq!(back, menu);
    }
}
