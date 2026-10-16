//! Cross-machine management and launch links normalized into one placement forest.

use std::collections::HashMap;
use std::collections::HashSet;

#[cfg(test)]
use super::Account;
use super::Agent;
use super::AgentRow;
use super::ChildKind;
use super::CodexThreadId;
use super::HierarchyEdge;
use super::HierarchyRole;
use super::Machine;
use super::SessionIdentity;
#[cfg(test)]
use super::account::AccountLabels;
#[cfg(test)]
use super::account::ConfigHome;
#[cfg(test)]
use super::classify;
#[cfg(test)]
use super::classify::ProcessEntry;
#[cfg(test)]
use super::classify::SessionRecord;
use super::tree;
use super::tree::DispatchFold;
#[cfg(test)]
use super::tree::ThreadEntry;
#[cfg(test)]
use super::tree::TreeSources;
use super::worker_dispatch::DispatchLabel;
use crate::constants::HEADING_SEPARATOR;

/// A listed agent's identity across the shown machines.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RowIdentity<'a> {
    /// The machine that owns the process id.
    pub(crate) machine: &'a str,
    /// The agent's process on that machine.
    pub(crate) pid:     u32,
}

/// Why a listed agent occupies its place in the forest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Placement<'a> {
    /// The agent titles its own tile and appears in the summary.
    Root,
    /// A live Claude session manages the agent.
    Managed {
        /// The live manager selected by forest normalization.
        manager: RowIdentity<'a>,
        /// The responsibility stated by that accepted relationship.
        role:    HierarchyRole,
    },
    /// Its listed launcher holds the agent in the absence of a manager.
    Launched(RowIdentity<'a>),
}

impl<'a> Placement<'a> {
    /// Whether this placement names `parent`.
    pub(crate) fn is_child_of(self, parent: RowIdentity<'a>) -> bool {
        match self {
            Self::Root => false,
            Self::Managed { manager, .. } | Self::Launched(manager) => manager == parent,
        }
    }
}

/// Why a role is shown, or why the agent has no label.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum RoleLabel {
    /// No accepted management relationship identifies this agent's role.
    #[default]
    Unlabeled,
    /// The agent's accepted incoming edge states its role.
    Stated(HierarchyRole),
    /// Accepted outgoing relationships identify the manager's responsibility.
    Inferred(HierarchyRole),
}

impl RoleLabel {
    /// The role column of an agent line, including an unlabeled session.
    pub(crate) const fn column_text(self) -> &'static str {
        match self {
            Self::Unlabeled => "session",
            Self::Stated(role) | Self::Inferred(role) => role.text(),
        }
    }

    /// A suffix for the agent's name; `RoleLabel::Unlabeled` has none.
    pub(crate) fn suffix(self) -> String {
        match self {
            Self::Unlabeled => String::new(),
            Self::Stated(role) | Self::Inferred(role) => {
                format!("{HEADING_SEPARATOR}{}", role.text())
            },
        }
    }
}

/// One listed agent after cross-machine management and launcher resolution.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PlacedAgent<'a> {
    /// Machine and process, together unique among listed agents.
    pub(crate) identity:  RowIdentity<'a>,
    /// The machine's census row.
    pub(crate) row:       &'a AgentRow,
    /// Its normalized place in the forest.
    pub(crate) placement: super::Placement<'a>,
}

/// A thread moved from its app server's work to a manager's tile.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ManagedThread<'a> {
    /// The process whose children contain this thread, listed or transport-only.
    pub(crate) owner:       RowIdentity<'a>,
    /// The app server or listed ancestor carrying the thread on the wire.
    pub(crate) owner_row:   &'a AgentRow,
    /// Its index in that row's children.
    pub(crate) child_index: usize,
    /// The live Claude session managing the thread.
    pub(crate) manager:     RowIdentity<'a>,
    /// The responsibility stated by the accepted thread relationship.
    pub(crate) role:        HierarchyRole,
}

/// The one forest shared by tile ordering and summary membership.
#[derive(Debug)]
pub(crate) struct AgentPlacement<'a> {
    /// Every shown row, in machine and census order.
    pub(crate) rows:    Vec<super::PlacedAgent<'a>>,
    /// Threads whose hierarchy manager resolves.
    pub(crate) threads: Vec<super::ManagedThread<'a>>,
    /// Launcher-cycle rows promoted after the ordinary roots.
    rescued_roots:      HashSet<RowIdentity<'a>>,
    /// Shell/thread pairs joined after their tile membership is normalized.
    dispatch_folds:     Vec<DispatchFold<'a>>,
}

impl<'a> AgentPlacement<'a> {
    /// Resolve management links and launchers against every shown machine.
    pub(crate) fn from_machines(machines: &[Machine<'a>]) -> Self {
        let mut rows: Vec<_> = machines
            .iter()
            .flat_map(|machine| {
                machine
                    .state
                    .rows()
                    .iter()
                    .filter(|row| row.agent != Agent::CodexThreadHost)
                    .map(|row| PlacedAgent {
                        identity: RowIdentity {
                            machine: machine.name,
                            pid:     row.pid,
                        },
                        row,
                        placement: Placement::Root,
                    })
            })
            .collect();
        let identities: HashSet<_> = rows.iter().map(|row| row.identity).collect();
        for row in &mut rows {
            if let Some(pid) = row.row.launched_by {
                let launcher = RowIdentity {
                    machine: row.identity.machine,
                    pid,
                };
                if launcher != row.identity && identities.contains(&launcher) {
                    row.placement = Placement::Launched(launcher);
                }
            }
        }
        let mut edges: Vec<_> = machines
            .iter()
            .flat_map(|machine| machine.state.edges())
            .collect();
        edges.sort_by(|left, right| left.manager.cmp(&right.manager));
        let managers: HashMap<_, _> = rows
            .iter()
            .filter(|row| row.row.agent == Agent::Claude)
            .filter_map(|row| match &row.row.session_id {
                SessionIdentity::Claude(id) => Some((id.as_str(), row.identity)),
                SessionIdentity::Unrecorded => None,
            })
            .collect();
        Self::resolve_managers(&mut rows, &edges, &managers);
        let rescued_roots = Self::rescue_launcher_cycles(&mut rows);
        let threads = Self::resolve_threads(machines, &managers, &edges);
        let mut placement = Self {
            rows,
            threads,
            rescued_roots,
            dispatch_folds: Vec::new(),
        };
        placement.dispatch_folds = placement.resolve_dispatches(machines);
        placement
    }

    /// Find the tile root of a normalized listed member.
    fn tile_root(&self, mut identity: RowIdentity<'a>) -> RowIdentity<'a> {
        for _ in 0..self.rows.len() {
            match self
                .rows
                .iter()
                .find(|row| row.identity == identity)
                .map(|row| row.placement)
            {
                Some(Placement::Managed { manager, .. } | Placement::Launched(manager)) => {
                    identity = manager;
                },
                Some(Placement::Root) | None => break,
            }
        }
        identity
    }

    /// Include only visible work, assigning moved threads to their manager's tile.
    fn resolve_dispatches(&self, machines: &[Machine<'a>]) -> Vec<DispatchFold<'a>> {
        let mut children = Vec::new();
        for machine in machines {
            for row in machine.state.rows() {
                let owner = RowIdentity {
                    machine: machine.name,
                    pid:     row.pid,
                };
                let mut hidden_until = 0;
                for (index, child) in row.children.iter().enumerate() {
                    if index < hidden_until {
                        continue;
                    }
                    if matches!(child.kind, ChildKind::Session(_)) {
                        hidden_until = (index + 1..row.children.len())
                            .find(|&next| row.children[next].depth <= child.depth)
                            .unwrap_or(row.children.len());
                        continue;
                    }
                    let moved = self
                        .threads
                        .iter()
                        .find(|thread| thread.owner == owner && thread.child_index == index);
                    let tile = match moved {
                        Some(thread) => self.tile_root(thread.manager),
                        None if row.agent != Agent::CodexThreadHost => self.tile_root(owner),
                        None => continue,
                    };
                    children.push((tile, child));
                }
            }
        }
        tree::join_dispatches(&children)
    }

    /// Whether this shell's worker is already visible in its tile.
    pub(crate) fn folds_dispatch(&self, child: &super::ChildRow) -> bool {
        self.dispatch_folds
            .iter()
            .any(|fold| std::ptr::eq(fold.shell, child))
    }

    /// The dispatch label for a thread matched in its final tile.
    pub(crate) fn dispatch_label(&self, child: &super::ChildRow) -> DispatchLabel<'a> {
        self.dispatch_folds
            .iter()
            .find(|fold| std::ptr::eq(fold.thread, child))
            .map_or(DispatchLabel::Original, |fold| {
                DispatchLabel::Worker(fold.dispatch)
            })
    }

    /// Resolve `ManagedThread` entries, preferring listed owners over
    /// transport-only `Agent::CodexThreadHost` rows.
    fn resolve_threads(
        machines: &[Machine<'a>],
        managers: &HashMap<&str, RowIdentity<'a>>,
        edges: &[&HierarchyEdge],
    ) -> Vec<ManagedThread<'a>> {
        let mut thread_ids = HashSet::new();
        let mut owners: Vec<_> = machines
            .iter()
            .flat_map(|machine| {
                machine
                    .state
                    .rows()
                    .iter()
                    .map(move |row| (machine.name, row))
            })
            // Prefer the existing listed owner over an app-server carrier when
            // the process tree has already attached the same thread to an agent.
            .collect();
        owners.sort_by_key(|(_, row)| row.agent == Agent::CodexThreadHost);
        owners
            .into_iter()
            .flat_map(|(machine, row)| {
                row.children
                    .iter()
                    .enumerate()
                    .filter_map(move |(child_index, child)| {
                        if child.kind != ChildKind::Thread {
                            return None;
                        }
                        let CodexThreadId::Recorded(id) = &child.thread_id else {
                            return None;
                        };
                        edges
                            .iter()
                            .filter(|edge| edge.member == *id)
                            .find_map(|edge| {
                                managers
                                    .get(edge.manager.as_str())
                                    .map(|manager| (*manager, edge.role))
                            })
                            .map(|(manager, role)| ManagedThread {
                                owner: RowIdentity {
                                    machine,
                                    pid: row.pid,
                                },
                                owner_row: row,
                                child_index,
                                manager,
                                role,
                            })
                    })
            })
            .filter(
                |thread| match &thread.owner_row.children[thread.child_index].thread_id {
                    CodexThreadId::Recorded(id) => thread_ids.insert(id.as_str()),
                    CodexThreadId::Unrecorded => false,
                },
            )
            .collect()
    }

    /// Select every management link before inspecting the combined parent graph.
    fn resolve_managers(
        rows: &mut [PlacedAgent<'a>],
        edges: &[&HierarchyEdge],
        managers: &HashMap<&str, RowIdentity<'a>>,
    ) {
        let launchers: HashMap<_, _> = rows
            .iter()
            .map(|row| (row.identity, row.placement))
            .collect();
        for row in rows.iter_mut() {
            let SessionIdentity::Claude(member) = &row.row.session_id else {
                continue;
            };
            if let Some((manager, role)) = edges
                .iter()
                .filter(|edge| edge.member == *member)
                .filter_map(|edge| {
                    managers
                        .get(edge.manager.as_str())
                        .map(|manager| (*manager, edge.role))
                })
                .find(|(manager, _)| *manager != row.identity)
            {
                row.placement = Placement::Managed { manager, role };
            }
        }
        // Check candidates in session-id order, independent of machine/row order.
        // Dropping a cycle-closing link restores its original launch fallback;
        // repeat because that fallback can expose another management cycle.
        let mut candidates: Vec<_> = rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match &row.row.session_id {
                SessionIdentity::Claude(id) => Some((id.as_str(), index)),
                SessionIdentity::Unrecorded => None,
            })
            .collect();
        candidates.sort_unstable_by_key(|(id, _)| *id);
        loop {
            let mut dropped = false;
            for &(_, index) in &candidates {
                let Placement::Managed { manager, .. } = rows[index].placement else {
                    continue;
                };
                let mut ancestor = manager;
                let mut visited = HashSet::new();
                while visited.insert(ancestor) {
                    if ancestor == rows[index].identity {
                        rows[index].placement = launchers[&rows[index].identity];
                        dropped = true;
                        break;
                    }
                    match rows
                        .iter()
                        .find(|row| row.identity == ancestor)
                        .map(|row| row.placement)
                    {
                        Some(
                            Placement::Managed {
                                manager: parent, ..
                            }
                            | Placement::Launched(parent),
                        ) => {
                            ancestor = parent;
                        },
                        Some(Placement::Root) | None => break,
                    }
                }
            }
            if !dropped {
                break;
            }
        }
    }

    /// Keep launcher-only cycles visible in the same rescue order as the old cell traversal.
    fn rescue_launcher_cycles(rows: &mut [PlacedAgent<'a>]) -> HashSet<RowIdentity<'a>> {
        // Match the grid's visible-root rescue for launcher-only cycles:
        // visit ordinary roots first, then promote each unseen row in order.
        let mut visited = HashSet::new();
        let roots: Vec<_> = rows
            .iter()
            .filter(|row| row.placement == Placement::Root)
            .map(|row| row.identity)
            .collect();
        for root in roots {
            Self::visit(rows, root, &mut visited);
        }
        let mut rescued_roots = HashSet::new();
        for index in 0..rows.len() {
            if !visited.contains(&rows[index].identity) {
                rows[index].placement = Placement::Root;
                rescued_roots.insert(rows[index].identity);
                Self::visit(rows, rows[index].identity, &mut visited);
            }
        }
        rescued_roots
    }

    /// Tile roots in the established machine/group/age order, followed by cycle rescues.
    pub(crate) fn roots(&self) -> Vec<&PlacedAgent<'a>> {
        let mut roots = Vec::new();
        let mut machines = HashSet::new();
        for row in &self.rows {
            if !machines.insert(row.identity.machine) {
                continue;
            }
            let machine_roots = self.rows.iter().filter(|candidate| {
                candidate.identity.machine == row.identity.machine
                    && candidate.placement == Placement::Root
            });
            let (rescued, ordinary): (Vec<&PlacedAgent<'a>>, Vec<&PlacedAgent<'a>>) = machine_roots
                .partition(|candidate| self.rescued_roots.contains(&candidate.identity));
            let (groups, lone): (Vec<_>, Vec<_>) = ordinary.into_iter().partition(|candidate| {
                self.rows
                    .iter()
                    .any(|member| member.placement.is_child_of(candidate.identity))
                    || self
                        .threads
                        .iter()
                        .any(|thread| thread.manager == candidate.identity)
            });
            roots.extend(groups.into_iter().chain(lone).chain(rescued));
        }
        roots
    }

    /// The accepted incoming role wins over inference from managed members.
    pub(crate) fn role_label(&self, identity: RowIdentity<'a>) -> RoleLabel {
        if let Some(PlacedAgent {
            placement: Placement::Managed { role, .. },
            ..
        }) = self.rows.iter().find(|row| row.identity == identity)
        {
            return RoleLabel::Stated(*role);
        }
        let roles = self
            .rows
            .iter()
            .filter_map(|row| match row.placement {
                Placement::Managed { manager, role } if manager == identity => Some(role),
                Placement::Root | Placement::Managed { .. } | Placement::Launched(_) => None,
            })
            .chain(
                self.threads
                    .iter()
                    .filter(|thread| thread.manager == identity)
                    .map(|thread| thread.role),
            );
        roles.fold(RoleLabel::Unlabeled, |label, role| {
            if matches!(role, HierarchyRole::Unit | HierarchyRole::Showrunner) {
                RoleLabel::Inferred(HierarchyRole::Showrunner)
            } else if label == RoleLabel::Unlabeled {
                RoleLabel::Inferred(HierarchyRole::Unit)
            } else {
                label
            }
        })
    }

    /// Whether a listed row titles a tile and belongs in the summary.
    pub(crate) fn is_root(&self, machine: &str, pid: u32) -> bool {
        self.rows.iter().any(|row| {
            row.identity.machine == machine
                && row.identity.pid == pid
                && row.placement == Placement::Root
        })
    }

    /// Mark the descendants of a visible root without following a cycle twice.
    fn visit(
        rows: &[PlacedAgent<'a>],
        root: RowIdentity<'a>,
        visited: &mut HashSet<RowIdentity<'a>>,
    ) {
        if !visited.insert(root) {
            return;
        }
        for row in rows.iter().filter(|row| row.placement.is_child_of(root)) {
            Self::visit(rows, row.identity, visited);
        }
    }
}

/// Production classification and tree inputs for a mesh server without `CLAUDE_PID`.
#[cfg(test)]
pub(crate) fn mesh_worker_fixture() -> (super::MachineState, super::MachineState) {
    let process = |pid, parent, program: &str, arguments: &[&str]| ProcessEntry {
        pid,
        parent: Some(parent),
        name: program.to_string(),
        started: u64::from(pid),
        arguments: arguments.iter().map(|arg| (*arg).to_string()).collect(),
        directory: None,
        config_home: ConfigHome::Unread,
        claude_pid: None,
    };
    let manager_processes = [process(10, 1, "claude", &["claude"])];
    let sessions = [SessionRecord {
        pid:        10,
        session_id: "unit".to_string(),
        cwd:        None,
        name:       Some("unit".to_string()),
        status:     Some("busy".to_string()),
        entrypoint: None,
    }];
    let manager_rows = classify::agent_rows(
        &manager_processes,
        &sessions,
        &[],
        None,
        &AccountLabels::default(),
    );
    let processes = [
        process(20, 1, "python3", &["python3", "mesh_daemon.py"]),
        process(
            30,
            20,
            "codex",
            &["codex", "app-server", "--listen", "stdio://"],
        ),
    ];
    let mut worker_rows =
        classify::agent_rows(&processes, &[], &[], None, &AccountLabels::default());
    let sources = TreeSources {
        threads: HashMap::from([(
            30,
            vec![ThreadEntry {
                thread_state: super::ThreadState::Idle,
                thread_id:    CodexThreadId::Recorded("mesh-worker".to_string()),
                name:         "mesh worker".to_string(),
                started:      40,
                service_tier: super::ServiceTier::Fast,
                account:      Account::Unrecorded,
            }],
        )]),
        ..TreeSources::default()
    };
    tree::attach_children(&mut worker_rows, &processes, &sources);
    (
        super::MachineState::Answered(manager_rows, super::HierarchyEdges::default()),
        super::MachineState::Answered(
            worker_rows,
            super::HierarchyEdges(vec![HierarchyEdge {
                manager: "unit".to_string(),
                member:  "mesh-worker".to_string(),
                role:    super::HierarchyRole::Impl,
            }]),
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::census::Account;
    use crate::census::CensusUpdate;
    use crate::census::HierarchyEdges;
    use crate::census::HierarchyRole;
    use crate::census::MachineState;
    use crate::census::ServiceTier;

    fn agent(pid: u32, id: &str) -> AgentRow {
        AgentRow {
            agent: Agent::Claude,
            session_id: SessionIdentity::Claude(id.to_string()),
            service_tier: ServiceTier::Unrecorded,
            account: Account::Unrecorded,
            name: id.to_string(),
            status: None,
            started: u64::from(pid),
            process_started: u64::from(pid),
            pid,
            desktop: None,
            directory: String::new(),
            branch: None,
            launched_by: None,
            children: Vec::new(),
        }
    }

    /// Accepted incoming roles beat inference, while launch and rejected edges label nothing.
    #[test]
    fn labels_use_only_accepted_links_and_explicit_roles_beat_inference() {
        let state = MachineState::Answered(
            vec![
                agent(1, "production"),
                agent(2, "unit"),
                agent(3, "worker"),
                AgentRow {
                    launched_by: Some(1),
                    ..agent(4, "launch-only")
                },
                agent(5, "unresolved"),
                agent(6, "self-linked"),
            ],
            HierarchyEdges(vec![
                edge("production", "unit", HierarchyRole::Impl),
                edge("unit", "worker", HierarchyRole::Showrunner),
                edge("absent", "unresolved", HierarchyRole::Review),
                edge("self-linked", "self-linked", HierarchyRole::Fix),
            ]),
        );
        let placement = AgentPlacement::from_machines(&[Machine {
            name:  "local",
            state: &state,
        }]);
        let labels: Vec<_> = placement
            .rows
            .iter()
            .map(|row| (row.row.name.as_str(), placement.role_label(row.identity)))
            .collect();
        assert_eq!(
            labels,
            [
                ("production", RoleLabel::Inferred(HierarchyRole::Unit)),
                ("unit", RoleLabel::Stated(HierarchyRole::Impl)),
                ("worker", RoleLabel::Stated(HierarchyRole::Showrunner)),
                ("launch-only", RoleLabel::Unlabeled),
                ("unresolved", RoleLabel::Unlabeled),
                ("self-linked", RoleLabel::Unlabeled),
            ]
        );
    }

    fn edge(manager: &str, member: &str, role: HierarchyRole) -> HierarchyEdge {
        HierarchyEdge {
            manager: manager.to_string(),
            member: member.to_string(),
            role,
        }
    }

    /// A rejected alternate manager contributes neither its stated role nor an inferred label.
    #[test]
    fn conflicting_roles_label_only_the_selected_manager_and_member() {
        let state = MachineState::Answered(
            vec![agent(1, "a"), agent(2, "b"), agent(3, "worker")],
            HierarchyEdges(vec![
                edge("missing", "worker", HierarchyRole::Fix),
                edge("b", "worker", HierarchyRole::Unit),
                edge("a", "worker", HierarchyRole::Test),
            ]),
        );
        let placement = AgentPlacement::from_machines(&[Machine {
            name:  "local",
            state: &state,
        }]);
        assert_eq!(
            placement
                .rows
                .iter()
                .map(|row| placement.role_label(row.identity))
                .collect::<Vec<_>>(),
            [
                RoleLabel::Inferred(HierarchyRole::Unit),
                RoleLabel::Unlabeled,
                RoleLabel::Stated(HierarchyRole::Test),
            ]
        );
    }

    /// Both agent and thread links infer manager roles; removing answers clears those labels.
    #[test]
    fn labels_follow_current_roles_and_relationship_liveness() {
        let (manager, worker) = mesh_worker_fixture();
        for (edges, expected_manager, expected_thread) in [
            (
                worker.edges().to_vec(),
                RoleLabel::Inferred(HierarchyRole::Unit),
                vec![HierarchyRole::Impl],
            ),
            (
                vec![edge("unit", "mesh-worker", HierarchyRole::Unit)],
                RoleLabel::Inferred(HierarchyRole::Showrunner),
                vec![HierarchyRole::Unit],
            ),
            (
                vec![edge("missing", "mesh-worker", HierarchyRole::Test)],
                RoleLabel::Unlabeled,
                vec![],
            ),
            (vec![], RoleLabel::Unlabeled, vec![]),
        ] {
            let worker = MachineState::Answered(worker.rows().to_vec(), HierarchyEdges(edges));
            let placement = AgentPlacement::from_machines(&[
                Machine {
                    name:  "natedev",
                    state: &manager,
                },
                Machine {
                    name:  "mac",
                    state: &worker,
                },
            ]);
            assert_eq!(
                placement.role_label(placement.rows[0].identity),
                expected_manager
            );
            assert_eq!(
                placement
                    .threads
                    .iter()
                    .map(|thread| thread.role)
                    .collect::<Vec<_>>(),
                expected_thread
            );
        }
        let state = MachineState::Answered(
            vec![agent(1, "a"), agent(2, "b")],
            HierarchyEdges(vec![
                edge("b", "a", HierarchyRole::Review),
                edge("a", "b", HierarchyRole::Unit),
            ]),
        );
        let placement = AgentPlacement::from_machines(&[Machine {
            name:  "local",
            state: &state,
        }]);
        assert_eq!(
            placement.role_label(placement.rows[0].identity),
            RoleLabel::Inferred(HierarchyRole::Showrunner)
        );
        assert_eq!(
            placement.role_label(placement.rows[1].identity),
            RoleLabel::Stated(HierarchyRole::Unit)
        );
    }

    /// Final management links replace transient launch links in every input order.
    #[test]
    fn a_management_chain_overrides_its_reverse_launcher_in_every_row_order() {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let original = [
                agent(1, "A"),
                AgentRow {
                    launched_by: Some(1),
                    ..agent(2, "B")
                },
                agent(3, "C"),
            ];
            let state = MachineState::Answered(
                order.map(|index| original[index].clone()).to_vec(),
                HierarchyEdges(vec![
                    HierarchyEdge {
                        manager: "C".to_string(),
                        member:  "B".to_string(),
                        role:    HierarchyRole::Unit,
                    },
                    HierarchyEdge {
                        manager: "B".to_string(),
                        member:  "A".to_string(),
                        role:    HierarchyRole::Impl,
                    },
                ]),
            );
            let machines = [Machine {
                name:  "local",
                state: &state,
            }];
            let placement = AgentPlacement::from_machines(&machines);
            let links: HashMap<_, _> = placement
                .rows
                .iter()
                .map(|row| (row.identity.pid, row.placement))
                .collect();
            assert_eq!(
                links[&1],
                Placement::Managed {
                    manager: RowIdentity {
                        machine: "local",
                        pid:     2,
                    },
                    role:    HierarchyRole::Impl,
                },
                "{order:?}"
            );
            assert_eq!(
                links[&2],
                Placement::Managed {
                    manager: RowIdentity {
                        machine: "local",
                        pid:     3,
                    },
                    role:    HierarchyRole::Unit,
                },
                "{order:?}"
            );
            assert_eq!(
                placement
                    .roots()
                    .iter()
                    .map(|row| row.identity.pid)
                    .collect::<Vec<_>>(),
                [3],
                "{order:?}"
            );
        }
    }

    /// A real management cycle drops the same link and restores its launcher in every order.
    #[test]
    fn cyclic_management_links_restore_the_same_launcher_in_every_row_order() {
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            let original = [
                AgentRow {
                    launched_by: Some(3),
                    ..agent(1, "A")
                },
                agent(2, "B"),
                agent(3, "C"),
            ];
            let state = MachineState::Answered(
                order.map(|index| original[index].clone()).to_vec(),
                HierarchyEdges(vec![
                    HierarchyEdge {
                        manager: "B".to_string(),
                        member:  "A".to_string(),
                        role:    HierarchyRole::Unit,
                    },
                    HierarchyEdge {
                        manager: "A".to_string(),
                        member:  "B".to_string(),
                        role:    HierarchyRole::Impl,
                    },
                ]),
            );
            let placement = AgentPlacement::from_machines(&[Machine {
                name:  "local",
                state: &state,
            }]);
            let links: HashMap<_, _> = placement
                .rows
                .iter()
                .map(|row| (row.identity.pid, row.placement))
                .collect();
            assert_eq!(
                links[&1],
                Placement::Launched(RowIdentity {
                    machine: "local",
                    pid:     3,
                }),
                "{order:?}"
            );
            assert_eq!(
                links[&2],
                Placement::Managed {
                    manager: RowIdentity {
                        machine: "local",
                        pid:     1,
                    },
                    role:    HierarchyRole::Impl,
                },
                "{order:?}"
            );
            assert_eq!(
                placement
                    .roots()
                    .iter()
                    .map(|row| row.identity.pid)
                    .collect::<Vec<_>>(),
                [3],
                "{order:?}"
            );
        }
    }

    /// The real mesh process shape retains workers without exposing its server as an agent.
    #[test]
    fn a_mesh_server_without_a_claude_pid_transports_managed_workers_only() {
        let (manager, worker) = mesh_worker_fixture();
        let machines = [
            Machine {
                name:  "natedev",
                state: &manager,
            },
            Machine {
                name:  "mac",
                state: &worker,
            },
        ];
        let placement = AgentPlacement::from_machines(&machines);
        assert_eq!(placement.rows.len(), 1);
        assert_eq!(placement.threads.len(), 1);
        assert_eq!(placement.threads[0].owner.machine, "mac");
        assert_eq!(
            placement.threads[0].owner_row.children[0].name,
            "mesh worker"
        );
        assert_eq!(
            placement.threads[0].manager,
            RowIdentity {
                machine: "natedev",
                pid:     10,
            }
        );
        let mut census = super::super::Census::new("mac".to_string());
        census.apply(CensusUpdate::Local(
            worker.rows().to_vec(),
            HierarchyEdges::default(),
        ));
        assert!(!census.lists_an_agent(&[]));
        let unlinked = MachineState::Answered(worker.rows().to_vec(), HierarchyEdges::default());
        let placement = AgentPlacement::from_machines(&[Machine {
            name:  "mac",
            state: &unlinked,
        }]);
        assert!(placement.roots().is_empty());
        assert!(placement.threads.is_empty());
    }
}
