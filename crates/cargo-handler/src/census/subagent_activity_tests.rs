//! Scan details carry each subagent transcript's own activity to its drawn row.

#![expect(
    clippy::expect_used,
    reason = "isolated transcript fixture files must exist"
)]

use std::io::Write;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::Value;
use tree::TreeSources;

use super::*;
use crate::agent_cell;
use crate::census::AccountColumn;
use crate::census::AgentPlacement;
use crate::census::ChildKind;
use crate::census::ChildRow;
use crate::census::HierarchyEdges;
use crate::census::Machine;
use crate::census::MachineState;
use crate::census::account::ConfigHome;
use crate::config::AccountsConfig;
use crate::footer_colors::FooterColors;
use crate::summary;

const ACTIVE_SUBAGENT_TURN: &str =
    "{\"type\":\"assistant\",\"message\":{\"stop_reason\":\"tool_use\"}}\n";
const FINISHED_SUBAGENT_TURN: &str =
    "{\"type\":\"assistant\",\"message\":{\"stop_reason\":\"end_turn\"}}\n";

#[test]
fn subagent_transcript_read_tool_reaches_the_drawn_status_and_append_updates_it() {
    let home = tempfile::tempdir().expect("fixture home");
    let cwd = Path::new("/fixture");
    let projects = home.path().join(".claude/projects");
    let session_path = transcript::transcript_path(&projects, cwd, "fixture-session");
    fs::create_dir_all(session_path.parent().expect("project directory"))
        .expect("project directories");
    fs::write(
        &session_path,
        "{\"type\":\"user\",\"message\":{\"content\":\"Inspect sources\"}}\n",
    )
    .expect("session transcript");
    let subagents = session_path.with_extension("").join("subagents");
    fs::create_dir_all(&subagents).expect("subagent directory");
    fs::write(
        subagents.join("agent-reader.meta.json"),
        "{\"description\":\"Inspect source files\"}",
    )
    .expect("metadata");
    let path = subagents.join("agent-reader.jsonl");
    fs::write(&path,"{\"type\":\"assistant\",\"message\":{\"stop_reason\":\"tool_use\",\"content\":[{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{\"file_path\":\"/fixture/input.rs\"}}]}}\n").expect("subagent transcript");
    let process = ProcessEntry {
        pid:         10,
        parent:      None,
        name:        "claude".into(),
        arguments:   vec!["claude".into()],
        started:     10,
        directory:   Some(cwd.into()),
        claude_pid:  None,
        config_home: ConfigHome::Unread,
    };
    let sessions: Vec<SessionRecord> = serde_json::from_value(
        serde_json::json!([{"pid":10,"sessionId":"fixture-session","cwd":cwd,"name":"director"}]),
    )
    .expect("session record");
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{"agent":"claude","name":"director","pid":10,"started":10,"directory":cwd,"children":[]}])).expect("session row");
    let mut scanner = LocalScanner::new(AccountLabels::new(&AccountsConfig::default(), None));
    scanner.desktop_source = DesktopSource::Windows(Vec::new());
    for expected in ["reading", "writing"] {
        scanner.attach_details(
            std::slice::from_ref(&process),
            &sessions,
            Some(home.path()),
            &mut rows,
        );
        assert_eq!(
            rows[0].children.len(),
            1,
            "subagent discovery: {:?}",
            rows[0].children
        );
        let state = MachineState::Answered(rows.clone(), HierarchyEdges::default().into());
        let machines = [Machine {
            name:  "fixture",
            state: &state,
        }];
        let placement = AgentPlacement::from_machines(&machines);
        let cells = agent_cell::cell_order_placed(&placement);
        let area = Rect::new(0, 0, 180, 20);
        let mut buffer = Buffer::empty(area);
        agent_cell::draw(
            &mut buffer,
            area,
            &cells[0],
            &cells,
            unix_now(),
            AccountColumn::Hidden,
            &FooterColors::default(),
        );
        let row = (0..area.height)
            .find(|&y| {
                (0..area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
                    .contains("Inspect source files")
            })
            .expect("drawn subagent row");
        let text: String = (0..area.width).map(|x| buffer[(x, row)].symbol()).collect();
        assert!(
            text.split_whitespace().any(|word| word == expected),
            "{text}"
        );
        writeln!(fs::OpenOptions::new().append(true).open(&path).expect("append transcript"),"{}",serde_json::json!({"type":"assistant","message":{"stop_reason":"tool_use","content":[{"type":"tool_use","name":"Edit","input":{}}]}})).expect("append tool");
    }
}

/// Metadata precedence and the description vocabulary, including an unreadable role.
const SUBAGENT_ROLE_CASES: [(&str, &str, &str, &str); 7] = [
    (
        "capture",
        "Capture the current screen",
        "general-purpose",
        "capture",
    ),
    (
        "review",
        "Review these changes",
        "general-purpose",
        "reviewer",
    ),
    ("lint", "Lint the crate", "general-purpose", "lint"),
    (
        "test",
        "Test the updated renderer",
        "general-purpose",
        "test",
    ),
    (
        "survey",
        "Survey the code paths",
        "general-purpose",
        "survey",
    ),
    ("typed", "Review the changed files", "capture", "capture"),
    ("unknown", "Complete this task", "general-purpose", ""),
];

#[test]
fn subagent_metadata_type_precedes_the_fixed_description_role_table_in_drawn_cells() {
    let home = tempfile::tempdir().expect("fixture project");
    let session = home.path().join("session.jsonl");
    let directory = home.path().join("session/subagents");
    fs::create_dir_all(&directory).expect("subagents");
    let cases = SUBAGENT_ROLE_CASES;
    for (id, description, agent_type, _) in cases {
        fs::write(
            directory.join(format!("agent-{id}.meta.json")),
            serde_json::json!({"description":description,"agentType":agent_type}).to_string(),
        )
        .expect("metadata");
        fs::write(
            directory.join(format!("agent-{id}.jsonl")),
            "{\"type\":\"assistant\",\"message\":{\"stop_reason\":\"tool_use\"}}\n",
        )
        .expect("transcript");
    }
    let subagents = transcript::running_subagents(
        &session,
        std::time::SystemTime::now(),
        &mut transcript::SubagentReads::default(),
    );
    assert_eq!(subagents.len(), cases.len());
    let process = ProcessEntry {
        pid:         10,
        parent:      None,
        name:        "claude".into(),
        arguments:   vec!["claude".into()],
        started:     10,
        directory:   None,
        claude_pid:  None,
        config_home: ConfigHome::Unread,
    };
    let mut rows:Vec<AgentRow>=serde_json::from_value(serde_json::json!([{"agent":"claude","name":"director","pid":10,"started":10,"directory":"/fixture","children":[]}])).expect("session row");
    tree::attach_children(
        &mut rows,
        &[process],
        &TreeSources {
            subagents: std::collections::HashMap::from([(10, subagents)]),
            ..tree::TreeSources::default()
        },
    );
    let state = MachineState::Answered(rows, HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = crate::census::AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let area = Rect::new(0, 0, 160, 20);
    let mut buffer = Buffer::empty(area);
    agent_cell::draw(
        &mut buffer,
        area,
        &cells[0],
        &cells,
        unix_now(),
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    let lines: Vec<String> = (0..area.height)
        .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let header = lines
        .iter()
        .find(|line| line.contains("session") && line.contains("status"))
        .expect("tree header");
    let start = usize::from(summary::cell_width(
        &header[..header.find("role").expect("role header")],
    ));
    let end = usize::from(summary::cell_width(
        &header[..header.find("status").expect("status header")],
    ));
    for (_, description, _, expected) in cases {
        let line = lines
            .iter()
            .find(|line| line.contains(description))
            .expect("subagent row");
        let cells: Vec<_> = line.chars().collect();
        assert_eq!(
            cells[start..end].iter().collect::<String>().trim(),
            expected,
            "{description}"
        );
    }
}

/// Read the real metadata boundary and attach its rows to the owning session.
fn metadata_children(records: &[(&str, Value, &str)]) -> Vec<ChildRow> {
    let home = tempfile::tempdir().expect("metadata fixture");
    let session = home.path().join("session.jsonl");
    let directory = home.path().join("session/subagents");
    fs::create_dir_all(&directory).expect("subagent directory");
    for (id, metadata, transcript) in records {
        fs::write(
            directory.join(format!("agent-{id}.meta.json")),
            metadata.to_string(),
        )
        .expect("metadata");
        fs::write(directory.join(format!("agent-{id}.jsonl")), transcript)
            .expect("subagent transcript");
    }
    let subagents = transcript::running_subagents(
        &session,
        std::time::SystemTime::now(),
        &mut transcript::SubagentReads::default(),
    );
    let mut rows: Vec<AgentRow> = serde_json::from_value(serde_json::json!([{
        "agent":"claude", "name":"director", "pid":10, "started":10,
        "directory":"/fixture", "children":[]
    }]))
    .expect("owning session");
    let process = ProcessEntry {
        pid:         10,
        parent:      None,
        name:        "claude".into(),
        arguments:   vec!["claude".into()],
        started:     10,
        directory:   None,
        claude_pid:  None,
        config_home: ConfigHome::Unread,
    };
    tree::attach_children(
        &mut rows,
        &[process],
        &TreeSources {
            subagents: std::collections::HashMap::from([(10, subagents)]),
            ..TreeSources::default()
        },
    );
    rows.remove(0).children
}

#[test]
fn named_subagent_metadata_draws_only_closed_role_words() {
    let cases = [
        ("writer77", "general-purpose", "writer"),
        ("writer78", "general-purpose", "writer"),
        ("p36-reviewer", "general-purpose", "reviewer"),
        ("review3", "general-purpose", "reviewer"),
        ("judge", "general-purpose", "agent"),
        ("", "Plan", "plan"),
    ];
    let records: Vec<_> = cases
        .iter()
        .enumerate()
        .map(|(index, (name, kind, _))| {
            (
                format!("case-{index}"),
                serde_json::json!({
                    "name":name, "agentType":kind, "spawnedAgentType":"general-purpose",
                    "description":format!("Task {index}")
                }),
            )
        })
        .collect();
    let inputs: Vec<_> = records
        .iter()
        .map(|(id, meta)| (id.as_str(), meta.clone(), ACTIVE_SUBAGENT_TURN))
        .collect();
    let children = metadata_children(&inputs);
    assert_eq!(children.len(), cases.len());
    let root: AgentRow = serde_json::from_value(serde_json::json!({
        "agent":"claude", "name":"director", "pid":10, "started":10,
        "directory":"/fixture", "children":children
    }))
    .expect("drawn owner");
    let state = MachineState::Answered(vec![root], HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let area = Rect::new(0, 0, 180, 20);
    let mut buffer = Buffer::empty(area);
    agent_cell::draw(
        &mut buffer,
        area,
        &cells[0],
        &cells,
        unix_now(),
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    let lines: Vec<String> = (0..area.height)
        .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let header = lines
        .iter()
        .find(|line| line.contains("session") && line.contains("status"))
        .expect("tree header");
    let role_start = usize::from(summary::cell_width(
        &header[..header.find("role").expect("role header")],
    ));
    let role_end = usize::from(summary::cell_width(
        &header[..header.find("status").expect("status header")],
    ));
    for (index, (name, _, expected)) in cases.iter().enumerate() {
        let label = format!("Task {index}");
        let line = lines
            .iter()
            .find(|line| line.contains(&label))
            .expect("named work row");
        let role: String = line
            .chars()
            .skip(role_start)
            .take(role_end - role_start)
            .collect();
        assert_eq!(role.trim(), *expected, "metadata name {name}: {line}");
    }
}

#[test]
fn subagent_role_sources_follow_name_type_spawned_type_and_description_order() {
    let cases = [
        (
            serde_json::json!({"name":"writer77", "agentType":"reviewer", "spawnedAgentType":"capture", "description":"Plan work"}),
            "writer",
        ),
        (
            serde_json::json!({"agentType":"review3", "spawnedAgentType":"capture", "description":"Plan work"}),
            "reviewer",
        ),
        (
            serde_json::json!({"agentType":"general-purpose", "spawnedAgentType":"Explore", "description":"Plan work"}),
            "explore",
        ),
        (
            serde_json::json!({"agentType":"general-purpose", "spawnedAgentType":"general-purpose", "description":"Review work"}),
            "reviewer",
        ),
        (
            serde_json::json!({"name":"copywriter", "description":"Plan work"}),
            "agent",
        ),
        (
            serde_json::json!({"name":"WRITER-78", "description":"Complete work"}),
            "writer",
        ),
        (
            serde_json::json!({"description":"Complete work", "agentType":"general-purpose"}),
            "",
        ),
    ];
    for (metadata, expected) in cases {
        let children = metadata_children(&[("work", metadata.clone(), ACTIVE_SUBAGENT_TURN)]);
        assert_eq!(children.len(), 1, "{metadata}");
        let ChildKind::Subagent(role) = &children[0].kind else {
            assert!(
                matches!(children[0].kind, ChildKind::Subagent(_)),
                "{children:?}"
            );
            return;
        };
        assert_eq!(role.text(), expected, "{metadata}");
    }
}

#[test]
fn absent_and_finished_subagent_parents_keep_work_under_the_owning_session() {
    let children = metadata_children(&[
        (
            "root",
            serde_json::json!({"description":"Root work", "name":"writer77"}),
            ACTIVE_SUBAGENT_TURN,
        ),
        (
            "finished",
            serde_json::json!({"description":"Finished parent", "name":"review3"}),
            FINISHED_SUBAGENT_TURN,
        ),
        (
            "orphan",
            serde_json::json!({"description":"Continuing work", "name":"writer78", "parentAgentId":"finished"}),
            ACTIVE_SUBAGENT_TURN,
        ),
        (
            "nested",
            serde_json::json!({"description":"Nested work", "name":"review3", "parentAgentId":"root"}),
            ACTIVE_SUBAGENT_TURN,
        ),
    ]);
    assert_eq!(children.len(), 3, "{children:?}");
    for name in ["Root work", "Continuing work"] {
        let child = children
            .iter()
            .find(|child| child.name == name)
            .expect("active work");
        assert_eq!(
            child.depth, 0,
            "{name} remains directly below the owning session"
        );
    }
    assert!(!children.iter().any(|child| child.name == "Finished parent"));
    let nested = children
        .iter()
        .find(|child| child.name == "Nested work")
        .expect("nested work");
    assert_eq!(nested.depth, 1, "an active parent keeps its child nested");
}

fn assert_metadata_role_drawn(metadata: &Value, expected: &str) {
    let description = metadata["description"].as_str().expect("work description");
    let children = metadata_children(&[
        ("work", metadata.clone(), ACTIVE_SUBAGENT_TURN),
        (
            "control",
            serde_json::json!({"name":"writer", "description":"Control work"}),
            ACTIVE_SUBAGENT_TURN,
        ),
    ]);
    let root: AgentRow = serde_json::from_value(serde_json::json!({
        "agent":"claude", "name":"director", "pid":10, "started":10,
        "directory":"/fixture", "children":children
    }))
    .expect("drawn owner");
    let state = MachineState::Answered(vec![root], HierarchyEdges::default().into());
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    let area = Rect::new(0, 0, 180, 20);
    let mut buffer = Buffer::empty(area);
    agent_cell::draw(
        &mut buffer,
        area,
        &cells[0],
        &cells,
        unix_now(),
        AccountColumn::Hidden,
        &FooterColors::default(),
    );
    let lines: Vec<String> = (0..area.height)
        .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect();
    let header = lines
        .iter()
        .find(|line| line.contains("session") && line.contains("status"))
        .expect("tree header");
    let role_start = usize::from(summary::cell_width(
        &header[..header.find("role").expect("role header")],
    ));
    let role_end = usize::from(summary::cell_width(
        &header[..header.find("status").expect("status header")],
    ));
    let line = lines
        .iter()
        .find(|line| line.contains(description))
        .expect("metadata work row");
    let role: String = line
        .chars()
        .skip(role_start)
        .take(role_end - role_start)
        .collect();
    assert_eq!(role.trim(), expected, "{metadata}: {line}");
}

#[test]
fn description_action_synonyms_draw_their_role_words() {
    for (description, expected) in [
        ("Reviewing phase 3", "reviewer"),
        ("Testing the parser", "test"),
        ("Tester for the parser", "test"),
        ("Capturing the screen", "capture"),
        ("Linting the changes", "lint"),
        ("Surveying the sources", "survey"),
    ] {
        assert_metadata_role_drawn(
            &serde_json::json!({"agentType":"general-purpose", "description":description}),
            expected,
        );
    }
}

#[test]
fn agent_in_a_description_keeps_the_role_cell_blank() {
    assert_metadata_role_drawn(
        &serde_json::json!({
            "agentType":"general-purpose", "description":"Find the agent cell code"
        }),
        "",
    );
}

#[test]
fn an_agent_name_draws_the_agent_role_word() {
    assert_metadata_role_drawn(
        &serde_json::json!({
            "name":"agent-7", "agentType":"general-purpose", "description":"Named work"
        }),
        "agent",
    );
}
