use std::collections::HashMap;
use std::collections::HashSet;

use toml::Table;
use toml::Value;

use crate::Action;
use crate::Bindings;
use crate::GlobalAction;
use crate::KeyBind;
use crate::KeyParseError;
use crate::KeymapError;

/// Walk `[scope_name]` in the parsed TOML table (if present) and
/// override matching actions in `defaults`. Returns the overlaid
/// `Bindings` (replace-per-action semantics: TOML keys for an action
/// fully replace the action's defaults).
pub(super) fn apply_toml_overlay<A>(
    scope_name: &str,
    defaults: Bindings<A>,
    table: Option<&Table>,
) -> Result<Bindings<A>, KeymapError>
where
    A: Action,
{
    apply_toml_overlay_with_peer(scope_name, defaults, table, None)
}

/// Same as [`apply_toml_overlay`], but unknown actions present in
/// `peer_action_keys` are treated as belonging to another enum that
/// shares the same TOML table. Used for the split `[global]` table.
pub(super) fn apply_toml_overlay_with_peer<A>(
    scope_name: &str,
    mut defaults: Bindings<A>,
    table: Option<&Table>,
    peer_action_keys: Option<&HashSet<&'static str>>,
) -> Result<Bindings<A>, KeymapError>
where
    A: Action,
{
    let Some(table) = table else {
        return Ok(defaults);
    };
    let Some(scope_value) = table.get(scope_name) else {
        return Ok(defaults);
    };
    let Some(scope_table) = scope_value.as_table() else {
        return Ok(defaults);
    };

    for (action_key, value) in scope_table {
        let Some(action) = A::from_toml_key(action_key) else {
            if peer_action_keys.is_some_and(|keys| keys.contains(action_key.as_str())) {
                continue;
            }
            return Err(KeymapError::UnknownAction {
                scope:  scope_name.to_string(),
                action: action_key.clone(),
            });
        };
        let keys = parse_toml_value(scope_name, action_key, value)?;
        check_in_array_duplicates(scope_name, action_key, &keys)?;
        defaults.override_action(&action, keys);
    }

    check_cross_action_collision(scope_name, &defaults)?;
    Ok(defaults)
}

pub(super) fn action_key_set<A: Action>() -> HashSet<&'static str> {
    A::ALL.iter().map(|action| action.toml_key()).collect()
}

pub(super) fn framework_global_action_key_set() -> HashSet<&'static str> {
    let mut keys = action_key_set::<GlobalAction>();
    keys.insert("settings");
    keys
}

/// Parse a TOML scope entry value into `Vec<KeyBind>`. Accepts a
/// single string or an array of strings.
fn parse_toml_value(scope: &str, action: &str, value: &Value) -> Result<Vec<KeyBind>, KeymapError> {
    match value {
        Value::String(s) => {
            let bind = KeyBind::parse(s).map_err(|source| KeymapError::InvalidBinding {
                scope: scope.to_string(),
                action: action.to_string(),
                source,
            })?;
            Ok(vec![bind])
        },
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let s = item.as_str().ok_or_else(|| KeymapError::InvalidBinding {
                    scope:  scope.to_string(),
                    action: action.to_string(),
                    source: KeyParseError::Empty,
                })?;
                let bind = KeyBind::parse(s).map_err(|source| KeymapError::InvalidBinding {
                    scope: scope.to_string(),
                    action: action.to_string(),
                    source,
                })?;
                out.push(bind);
            }
            Ok(out)
        },
        _ => Err(KeymapError::InvalidBinding {
            scope:  scope.to_string(),
            action: action.to_string(),
            source: KeyParseError::Empty,
        }),
    }
}

/// Reject duplicate keys inside a single TOML array. Surfaces
/// [`KeymapError::InArrayDuplicate`].
fn check_in_array_duplicates(
    scope: &str,
    action: &str,
    keys: &[KeyBind],
) -> Result<(), KeymapError> {
    for (i, key) in keys.iter().enumerate() {
        if keys[..i].iter().any(|k| k == key) {
            return Err(KeymapError::InArrayDuplicate {
                scope:  scope.to_string(),
                action: action.to_string(),
                key:    key.display(),
            });
        }
    }
    Ok(())
}

/// Reject two actions in the same scope sharing one key. Surfaces
/// [`KeymapError::CrossActionCollision`].
fn check_cross_action_collision<A: Action>(
    scope: &str,
    bindings: &Bindings<A>,
) -> Result<(), KeymapError> {
    let mut seen: HashMap<KeyBind, A> = HashMap::new();
    for (key, action) in bindings_entries(bindings) {
        if let Some(existing) = seen.get(key)
            && *existing != *action
        {
            return Err(KeymapError::CrossActionCollision {
                scope:   scope.to_string(),
                key:     key.display(),
                actions: (
                    existing.toml_key().to_string(),
                    action.toml_key().to_string(),
                ),
            });
        }
        seen.insert(*key, *action);
    }
    Ok(())
}

/// Borrow the entries of a [`Bindings`] in insertion order.
fn bindings_entries<A>(bindings: &Bindings<A>) -> impl Iterator<Item = (&KeyBind, &A)> {
    bindings.entries().iter().map(|(k, a)| (k, a))
}
