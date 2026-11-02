//! Process lifetimes and capture publications point to one row owner.

use std::collections::HashMap;
use std::collections::HashSet;

use crate::birth_stamp::LifetimeEvidence;
use crate::birth_stamp::ProcessLifetime;
use crate::census::CargoProcess;
use crate::census::InvocationId;
use crate::census::process_identity::ProcessIdentity;
use crate::census::scan::InvocationExecution;

/// The first observed identity remains the row key through every alias.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) struct InvocationKey(pub(super) InvocationId);

/// Each observed identity points to one row owner.
#[derive(Clone, Default)]
pub(super) struct InvocationOwners {
    aliases: HashMap<InvocationAlias, InvocationKey>,
    present: HashMap<u32, PresentOrigin>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
enum InvocationAlias {
    Evidence(InvocationId),
    Lifetime {
        pid:      u32,
        lifetime: ProcessLifetime,
    },
}

#[derive(Clone)]
struct PresentOrigin {
    key:      InvocationKey,
    lifetime: LifetimeEvidence,
}

impl InvocationOwners {
    /// Resolve all aliases before the roster creates or refreshes a row.
    pub(super) fn resolve(
        &mut self,
        rows: &mut [&mut CargoProcess],
    ) -> HashMap<InvocationId, InvocationId> {
        let mut resolved = HashMap::new();
        let mut present = HashMap::new();
        for row in rows.iter_mut() {
            let lifetime = origin_lifetime(row);
            let mut aliases = vec![InvocationAlias::Evidence(row.invocation_id.clone())];
            if let LifetimeEvidence::Available(lifetime) = &lifetime {
                aliases.push(InvocationAlias::Lifetime {
                    pid:      row.pid,
                    lifetime: lifetime.clone(),
                });
            }
            if let InvocationExecution::Executing { executor, .. } = &row.execution {
                aliases.push(InvocationAlias::Evidence(InvocationId::Process(
                    executor.clone(),
                )));
            }
            let key = aliases
                .iter()
                .find_map(|alias| self.aliases.get(alias))
                .cloned()
                .or_else(|| {
                    self.present
                        .get(&row.pid)
                        .filter(|previous| {
                            matches!(
                                (&previous.lifetime, &lifetime),
                                (LifetimeEvidence::Unavailable, _)
                                    | (_, LifetimeEvidence::Unavailable)
                            )
                        })
                        .map(|previous| previous.key.clone())
                })
                .unwrap_or_else(|| InvocationKey(row.invocation_id.clone()));
            for alias in aliases {
                self.aliases.insert(alias, key.clone());
            }
            present.insert(
                row.pid,
                PresentOrigin {
                    key: key.clone(),
                    lifetime,
                },
            );
            resolved.insert(row.invocation_id.clone(), key.0.clone());
            row.invocation_id = key.0;
        }
        self.present = present;
        resolved
    }

    /// Release aliases when their row leaves the roster.
    pub(super) fn retain(&mut self, keys: &HashSet<InvocationId>) {
        self.aliases.retain(|_, key| keys.contains(&key.0));
    }
}

fn origin_lifetime(process: &CargoProcess) -> LifetimeEvidence {
    match &process.execution {
        InvocationExecution::AwaitingExecutor(lifetime)
        | InvocationExecution::Executing {
            origin: lifetime, ..
        } => lifetime.clone(),
        InvocationExecution::RegistrationOnly => LifetimeEvidence::Unavailable,
        InvocationExecution::Command => match &process.invocation_id {
            InvocationId::Process(ProcessIdentity::Known { lifetime, .. }) => {
                LifetimeEvidence::Available(lifetime.clone())
            },
            InvocationId::Process(ProcessIdentity::Unavailable { .. })
            | InvocationId::Captured(_) => LifetimeEvidence::Unavailable,
        },
    }
}
