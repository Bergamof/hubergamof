//! Placement rules of the modules, checked when a configuration is saved
//! ([`violations`]) and enforced on stored and imported ones ([`normalize`]).
//!
//! A module states what it allows (`ModuleCapabilities`): being placed on a
//! key, and being configured several times. A configuration written before a
//! rule existed, or edited by hand, can break it; instead of refusing the
//! whole device, it is brought back to the rules and the changes reported.
//! Both functions share [`Rules`], so what is refused is what is corrected.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use hubergamof_module_api::ModuleCapabilities;
use hubergamof_protocol::{DeviceConfig, DialAssignment, ModuleInstanceId, ModuleKind};

use crate::config_store::{ModuleDescriptors, used_instances};

/// What [`normalize`] changed for one module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementChange {
    /// The module cannot be placed on a key: its keys were removed.
    RemovedFromKeys {
        /// Display name of the module.
        module: String,
    },
    /// The module allows a single instance: the others were merged into one,
    /// which its keys and dials now share.
    MergedInstances {
        /// Display name of the module.
        module: String,
    },
    /// The module allows a single key: the others were removed.
    RemovedDuplicateKey {
        /// Display name of the module.
        module: String,
    },
    /// A module has at most one dial: the others were emptied.
    RemovedDuplicateDial {
        /// Display name of the module.
        module: String,
    },
}

/// A rule a configuration breaks, naming the instance at fault.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlacementViolation {
    /// Instance placed where its module does not allow it.
    pub instance: ModuleInstanceId,
    /// What is wrong, for the user (English: it ends up in an error detail).
    pub reason: &'static str,
}

const NO_KEY: &str = "this module cannot be placed on a key";
const ONE_KEY: &str = "this module can have only one key";
const ONE_INSTANCE: &str = "this module can be configured only once";
const ONE_DIAL: &str = "this module can have only one dial";

/// The rules a configuration breaks, in the order of its keys then modules.
/// Kinds the descriptors do not know are not checked.
pub fn violations(
    config: &DeviceConfig,
    descriptors: &dyn ModuleDescriptors,
) -> Vec<PlacementViolation> {
    let rules = Rules::of(config, descriptors);
    let kinds = kinds_by_instance(config);
    let mut found = Vec::new();
    let mut keyed = HashSet::new();
    for id in &config.home {
        let Some(kind) = kinds.get(id) else { continue };
        if rules.keyless(kind).is_some() {
            found.push(violation(id, NO_KEY));
        } else if rules.single(kind).is_some() && !keyed.insert(id) {
            found.push(violation(id, ONE_KEY));
        }
    }
    let mut seen = HashSet::new();
    for module in &config.modules {
        if rules.single(&module.kind).is_some() && !seen.insert(&module.kind) {
            found.push(violation(&module.id, ONE_INSTANCE));
        }
    }
    let mut dialed = HashSet::new();
    for assignment in &config.dials {
        if let DialAssignment::Module { instance, .. } = assignment
            && let Some(kind) = kinds.get(instance)
            && !dialed.insert(kind)
        {
            found.push(violation(instance, ONE_DIAL));
        }
    }
    found
}

fn violation(instance: &ModuleInstanceId, reason: &'static str) -> PlacementViolation {
    PlacementViolation {
        instance: instance.clone(),
        reason,
    }
}

/// Brings `config` back to the rules and returns what was changed. Kinds the
/// descriptors do not know are left alone (they show a placeholder).
///
/// Duplicates are merged into one instance per kind, the one on a key when
/// there is one (a key carries a label and an image, a dial does not): keys
/// and dials pointing at the others point at it, so the settings are shared.
/// A module has one dial at most, whatever it allows otherwise: the later
/// dials are emptied.
///
/// Instances the changes leave without a key or a dial are dropped; those
/// that already were unused are left as they are.
pub fn normalize(
    config: &mut DeviceConfig,
    descriptors: &dyn ModuleDescriptors,
) -> Vec<PlacementChange> {
    let rules = Rules::of(config, descriptors);
    let used_before = used_instances(config);
    let mut changes = Vec::new();
    merge_instances(config, &rules, &mut changes);
    remove_duplicate_keys(config, &rules, &mut changes);
    remove_keys_of_keyless_modules(config, &rules, &mut changes);
    remove_duplicate_dials(config, &rules, &mut changes);
    if !changes.is_empty() {
        let used_after = used_instances(config);
        config
            .modules
            .retain(|module| used_after.contains(&module.id) || !used_before.contains(&module.id));
    }
    changes
}

/// What a module of the configuration allows, and how to name it.
struct Rule {
    name: String,
    capabilities: ModuleCapabilities,
}

/// What each kind present in a configuration allows.
struct Rules(HashMap<ModuleKind, Rule>);

impl Rules {
    fn of(config: &DeviceConfig, source: &dyn ModuleDescriptors) -> Self {
        let mut rules = HashMap::new();
        for module in &config.modules {
            if rules.contains_key(&module.kind) {
                continue;
            }
            // Module code: a panic leaves the kind unchecked, like an unknown one.
            let descriptor = catch_unwind(AssertUnwindSafe(|| source.descriptor(&module.kind)))
                .unwrap_or_else(|_| {
                    tracing::error!(kind = %module.kind, "module panicked describing itself");
                    None
                });
            if let Some(descriptor) = descriptor {
                rules.insert(
                    module.kind.clone(),
                    Rule {
                        name: descriptor.name,
                        capabilities: descriptor.capabilities,
                    },
                );
            }
        }
        Self(rules)
    }

    /// The module of `kind` when it allows a single instance.
    fn single(&self, kind: &ModuleKind) -> Option<&Rule> {
        self.0
            .get(kind)
            .filter(|rule| !rule.capabilities.multiple_instances)
    }

    /// The module of `kind` when it cannot be on a key.
    fn keyless(&self, kind: &ModuleKind) -> Option<&Rule> {
        self.0.get(kind).filter(|rule| !rule.capabilities.key)
    }
}

fn record(changes: &mut Vec<PlacementChange>, change: PlacementChange) {
    if !changes.contains(&change) {
        changes.push(change);
    }
}

/// One instance per kind that allows only one; keys and dials follow.
fn merge_instances(config: &mut DeviceConfig, rules: &Rules, changes: &mut Vec<PlacementChange>) {
    let kinds = kinds_by_instance(config);
    let mut kept: HashMap<ModuleKind, ModuleInstanceId> = HashMap::new();
    let on_keys = config
        .home
        .iter()
        .filter_map(|id| Some((kinds.get(id)?, id)));
    let all = config
        .modules
        .iter()
        .map(|module| (&module.kind, &module.id));
    for (kind, id) in on_keys.chain(all) {
        if rules.single(kind).is_some() {
            kept.entry(kind.clone()).or_insert_with(|| id.clone());
        }
    }
    let mut merged: HashMap<ModuleInstanceId, ModuleInstanceId> = HashMap::new();
    for module in &config.modules {
        let (Some(rule), Some(keeper)) = (rules.single(&module.kind), kept.get(&module.kind))
        else {
            continue;
        };
        if *keeper != module.id {
            merged.insert(module.id.clone(), keeper.clone());
            record(
                changes,
                PlacementChange::MergedInstances {
                    module: rule.name.clone(),
                },
            );
        }
    }
    config
        .modules
        .retain(|module| !merged.contains_key(&module.id));
    for id in &mut config.home {
        if let Some(keeper) = merged.get(id) {
            id.clone_from(keeper);
        }
    }
    for dial in &mut config.dials {
        if let DialAssignment::Module { instance, .. } = dial
            && let Some(keeper) = merged.get(instance)
        {
            instance.clone_from(keeper);
        }
    }
}

/// A module allowing one instance has one key at most.
fn remove_duplicate_keys(
    config: &mut DeviceConfig,
    rules: &Rules,
    changes: &mut Vec<PlacementChange>,
) {
    let kinds = kinds_by_instance(config);
    let mut placed = HashSet::new();
    config.home.retain(|id| {
        let Some(rule) = kinds.get(id).and_then(|kind| rules.single(kind)) else {
            return true;
        };
        let first_key = placed.insert(id.clone());
        if !first_key {
            record(
                changes,
                PlacementChange::RemovedDuplicateKey {
                    module: rule.name.clone(),
                },
            );
        }
        first_key
    });
}

fn remove_keys_of_keyless_modules(
    config: &mut DeviceConfig,
    rules: &Rules,
    changes: &mut Vec<PlacementChange>,
) {
    let kinds = kinds_by_instance(config);
    config.home.retain(|id| {
        let Some(rule) = kinds.get(id).and_then(|kind| rules.keyless(kind)) else {
            return true;
        };
        record(
            changes,
            PlacementChange::RemovedFromKeys {
                module: rule.name.clone(),
            },
        );
        false
    });
}

/// Name of the module of `kind`, the kind itself when it is unknown.
fn name_of(rules: &Rules, kind: &ModuleKind) -> String {
    rules
        .0
        .get(kind)
        .map_or_else(|| kind.0.clone(), |rule| rule.name.clone())
}

fn remove_duplicate_dials(
    config: &mut DeviceConfig,
    rules: &Rules,
    changes: &mut Vec<PlacementChange>,
) {
    let kinds = kinds_by_instance(config);
    let mut dialed = HashSet::new();
    for assignment in &mut config.dials {
        let DialAssignment::Module { instance, .. } = assignment else {
            continue;
        };
        let Some(kind) = kinds.get(instance) else {
            continue;
        };
        if !dialed.insert(kind.clone()) {
            record(
                changes,
                PlacementChange::RemovedDuplicateDial {
                    module: name_of(rules, kind),
                },
            );
            *assignment = DialAssignment::None;
        }
    }
}

fn kinds_by_instance(config: &DeviceConfig) -> HashMap<ModuleInstanceId, ModuleKind> {
    config
        .modules
        .iter()
        .map(|module| (module.id.clone(), module.kind.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use hubergamof_protocol::ModuleInstance;
    use serde_json::json;

    use super::*;
    use crate::config_store::default_config;
    use crate::device::virtual_deck::virtual_plus_info;
    use crate::testing::builtin_registry;

    fn config(edit: impl FnOnce(&mut DeviceConfig)) -> DeviceConfig {
        let mut config = default_config(&virtual_plus_info("S"), &builtin_registry());
        edit(&mut config);
        config
    }

    fn instance(id: &str, kind: &str) -> ModuleInstance {
        ModuleInstance {
            id: id.into(),
            kind: kind.into(),
            label: None,
            image: None,
            settings: json!({}),
        }
    }

    fn dial(instance: &str) -> DialAssignment {
        DialAssignment::Module {
            instance: instance.into(),
            function: "f".into(),
        }
    }

    fn reasons(config: &DeviceConfig) -> Vec<&'static str> {
        violations(config, &builtin_registry())
            .into_iter()
            .map(|violation| violation.reason)
            .collect()
    }

    /// Configurations breaking each rule, and the ones breaking several.
    fn broken() -> Vec<DeviceConfig> {
        vec![
            config(|c| c.home.push("pomodoro".into())),
            config(|c| {
                c.modules.push(instance("audio-2", "audio"));
                c.home.push("audio-2".into());
            }),
            config(|c| c.home.push("claude".into())),
            config(|c| c.dials[2] = c.dials[0].clone()),
            config(|c| {
                c.modules.push(instance("docker-2", "docker"));
                c.dials[0] = dial("docker");
                c.dials[1] = dial("docker-2");
            }),
            config(|c| {
                c.modules.push(instance("pomodoro-2", "pomodoro"));
                c.home.push("pomodoro-2".into());
                c.dials[3] = dial("pomodoro-2");
            }),
        ]
    }

    #[test]
    fn the_default_configuration_breaks_no_rule() {
        let config = config(|_| {});
        assert_eq!(violations(&config, &builtin_registry()), []);
        let mut normalized = config.clone();
        assert_eq!(normalize(&mut normalized, &builtin_registry()), []);
        assert_eq!(normalized, config);
    }

    #[test]
    fn each_rule_is_reported() {
        assert_eq!(reasons(&broken()[0]), [NO_KEY]);
        assert_eq!(reasons(&broken()[1]), [ONE_INSTANCE]);
        assert_eq!(reasons(&broken()[2]), [ONE_KEY]);
        assert_eq!(reasons(&broken()[3]), [ONE_DIAL]);
        // A module that allows several instances still has one dial only.
        assert_eq!(reasons(&broken()[4]), [ONE_DIAL]);
    }

    #[test]
    fn what_is_normalized_breaks_no_rule_and_stays_put() {
        for mut config in broken() {
            let changes = normalize(&mut config, &builtin_registry());
            assert_ne!(changes, []);
            assert_eq!(reasons(&config), Vec::<&str>::new());
            let once = config.clone();
            assert_eq!(normalize(&mut config, &builtin_registry()), []);
            assert_eq!(config, once);
        }
    }

    #[test]
    fn the_later_dial_of_a_module_is_emptied_with_its_instance() {
        let mut config = broken().swap_remove(4);
        let changes = normalize(&mut config, &builtin_registry());
        assert_eq!(
            changes,
            [PlacementChange::RemovedDuplicateDial {
                module: "Docker".into()
            }]
        );
        assert_eq!(config.dials[0], dial("docker"));
        assert_eq!(config.dials[1], DialAssignment::None);
        assert!(!config.modules.iter().any(|m| m.id.0 == "docker-2"));
    }

    #[test]
    fn merging_keeps_the_instance_on_a_key_with_its_label() {
        let mut config = config(|c| {
            c.dials[2] = dial("audio-2");
            c.modules.push(ModuleInstance {
                label: Some("Son".into()),
                ..instance("audio-2", "audio")
            });
            c.home = vec!["audio-2".into()];
            c.modules.retain(|m| m.id.0 != "audio");
            c.modules.push(instance("audio", "audio"));
        });
        normalize(&mut config, &builtin_registry());
        let kept: Vec<_> = config
            .modules
            .iter()
            .filter(|m| m.kind.0 == "audio")
            .collect();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id.0, "audio-2");
        assert_eq!(kept[0].label.as_deref(), Some("Son"));
        assert_eq!(config.dials[0], dial_of("audio-2", "volume"));
    }

    fn dial_of(instance: &str, function: &str) -> DialAssignment {
        DialAssignment::Module {
            instance: instance.into(),
            function: function.into(),
        }
    }

    #[test]
    fn a_keyless_module_and_a_duplicate_are_both_reported_in_order() {
        let mut config = config(|c| {
            c.modules.push(instance("pomodoro-2", "pomodoro"));
            c.home.push("pomodoro-2".into());
        });
        let changes = normalize(&mut config, &builtin_registry());
        assert_eq!(
            changes,
            [
                PlacementChange::MergedInstances {
                    module: "Pomodoro".into()
                },
                PlacementChange::RemovedFromKeys {
                    module: "Pomodoro".into()
                }
            ]
        );
        assert!(!config.home.iter().any(|id| id.0.starts_with("pomodoro")));
        // The instance that was on a key is kept, and the dial follows it.
        assert_eq!(config.dials[1], dial_of("pomodoro-2", "control"));
        assert_eq!(
            config
                .modules
                .iter()
                .filter(|m| m.kind.0 == "pomodoro")
                .count(),
            1
        );
    }

    #[test]
    fn unknown_kinds_and_earlier_orphans_are_left_alone() {
        let mut config = config(|c| {
            c.modules.push(instance("orphan", "teleporter"));
            c.modules.push(instance("a", "teleporter"));
            c.modules.push(instance("b", "teleporter"));
            c.home.extend(["a".into(), "b".into()]);
            c.home.push("pomodoro".into());
        });
        let changes = normalize(&mut config, &builtin_registry());
        assert_eq!(changes.len(), 1);
        let ids: Vec<_> = config.modules.iter().map(|m| m.id.0.as_str()).collect();
        assert!(ids.contains(&"orphan") && ids.contains(&"a") && ids.contains(&"b"));
        assert!(
            config.home.iter().any(|id| id.0 == "a") && config.home.iter().any(|id| id.0 == "b")
        );
    }
}
