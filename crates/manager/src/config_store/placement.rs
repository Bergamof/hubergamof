//! Placement rules applied to stored and imported configurations.
//!
//! A module states what it allows (`ModuleCapabilities`): being placed on a
//! key, and being configured several times. A configuration written before a
//! rule existed, or edited by hand, can break it; instead of refusing the
//! whole device, it is brought back to the rules and the changes are reported.

use std::collections::{HashMap, HashSet};

use hubergamof_module_api::ModuleDescriptor;
use hubergamof_protocol::{DeviceConfig, DialAssignment, ModuleInstanceId, ModuleKind};

use super::ModuleDescriptors;

/// What [`normalize_placement`] changed for one module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlacementChange {
    /// The module cannot be placed on a key: its keys were removed.
    RemovedFromKeys {
        /// Display name of the module.
        module: String,
    },
    /// The module allows a single instance: the others were merged into the
    /// first one, which its keys and dials now share.
    MergedInstances {
        /// Display name of the module.
        module: String,
    },
}

/// Brings `config` back to the placement rules of its modules and returns
/// what was changed, in module order. Kinds the descriptors do not know are
/// left alone (they show a placeholder).
///
/// Duplicates are merged into the first instance of the kind: keys and dials
/// pointing at the others point at it, so the settings are shared. Instances
/// left without a key or a dial are dropped.
pub(super) fn normalize_placement(
    config: &mut DeviceConfig,
    descriptors: &dyn ModuleDescriptors,
) -> Vec<PlacementChange> {
    let rules = Rules::of(config, descriptors);
    let mut changes = Changes::default();
    merge_instances(config, &rules, &mut changes);
    remove_keys(config, &rules, &mut changes);
    // Only after a change: a configuration that follows the rules is kept as
    // it was written.
    if !changes.found.is_empty() {
        drop_unused_instances(config);
    }
    changes.into_vec()
}

/// What each kind present in a configuration allows.
struct Rules {
    descriptors: HashMap<ModuleKind, ModuleDescriptor>,
}

impl Rules {
    fn of(config: &DeviceConfig, source: &dyn ModuleDescriptors) -> Self {
        let mut descriptors = HashMap::new();
        for module in &config.modules {
            if !descriptors.contains_key(&module.kind)
                && let Some(descriptor) = source.descriptor(&module.kind)
            {
                descriptors.insert(module.kind.clone(), descriptor);
            }
        }
        Self { descriptors }
    }

    fn single_instance(&self, kind: &ModuleKind) -> Option<&ModuleDescriptor> {
        self.descriptors
            .get(kind)
            .filter(|descriptor| !descriptor.capabilities.multiple_instances)
    }

    fn without_keys(&self, kind: &ModuleKind) -> Option<&ModuleDescriptor> {
        self.descriptors
            .get(kind)
            .filter(|descriptor| !descriptor.capabilities.key)
    }
}

/// Changes in the order they were found, one per module and kind of change.
#[derive(Default)]
struct Changes {
    found: Vec<PlacementChange>,
}

impl Changes {
    fn record(&mut self, change: PlacementChange) {
        if !self.found.contains(&change) {
            self.found.push(change);
        }
    }

    fn into_vec(self) -> Vec<PlacementChange> {
        self.found
    }
}

/// One instance per kind that allows only one; keys and dials follow.
fn merge_instances(config: &mut DeviceConfig, rules: &Rules, changes: &mut Changes) {
    let mut kept: HashMap<ModuleKind, ModuleInstanceId> = HashMap::new();
    let mut merged: HashMap<ModuleInstanceId, ModuleInstanceId> = HashMap::new();
    for module in &config.modules {
        let Some(descriptor) = rules.single_instance(&module.kind) else {
            continue;
        };
        match kept.get(&module.kind) {
            None => {
                kept.insert(module.kind.clone(), module.id.clone());
            }
            Some(first) => {
                merged.insert(module.id.clone(), first.clone());
                changes.record(PlacementChange::MergedInstances {
                    module: descriptor.name.clone(),
                });
            }
        }
    }
    config
        .modules
        .retain(|module| !merged.contains_key(&module.id));
    for id in &mut config.home {
        if let Some(first) = merged.get(id) {
            *id = first.clone();
        }
    }
    for dial in &mut config.dials {
        if let DialAssignment::Module { instance, .. } = dial
            && let Some(first) = merged.get(instance)
        {
            *instance = first.clone();
        }
    }
    keep_one_key_per_single_instance(config, rules, changes);
}

/// A module allowing one instance has one key at most.
fn keep_one_key_per_single_instance(
    config: &mut DeviceConfig,
    rules: &Rules,
    changes: &mut Changes,
) {
    let kinds = kinds_by_instance(config);
    let mut placed = HashSet::new();
    config.home.retain(|id| {
        let Some(descriptor) = kinds.get(id).and_then(|kind| rules.single_instance(kind)) else {
            return true;
        };
        let first_key = placed.insert(id.clone());
        if !first_key {
            changes.record(PlacementChange::MergedInstances {
                module: descriptor.name.clone(),
            });
        }
        first_key
    });
}

/// Keys of the modules that cannot be on a key are removed.
fn remove_keys(config: &mut DeviceConfig, rules: &Rules, changes: &mut Changes) {
    let kinds = kinds_by_instance(config);
    config.home.retain(|id| {
        let Some(descriptor) = kinds.get(id).and_then(|kind| rules.without_keys(kind)) else {
            return true;
        };
        changes.record(PlacementChange::RemovedFromKeys {
            module: descriptor.name.clone(),
        });
        false
    });
}

fn kinds_by_instance(config: &DeviceConfig) -> HashMap<ModuleInstanceId, ModuleKind> {
    config
        .modules
        .iter()
        .map(|module| (module.id.clone(), module.kind.clone()))
        .collect()
}

fn drop_unused_instances(config: &mut DeviceConfig) {
    let used: HashSet<ModuleInstanceId> = config
        .home
        .iter()
        .chain(config.dials.iter().filter_map(DialAssignment::instance))
        .cloned()
        .collect();
    config.modules.retain(|module| used.contains(&module.id));
}
