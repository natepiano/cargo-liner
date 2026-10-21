//! Scan details carry each subagent transcript's own activity to its drawn row.

#![expect(
    clippy::expect_used,
    reason = "isolated transcript fixture files must exist"
)]

use std::io::Write;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use tree::TreeSources;

use super::*;
use crate::agent_cell;
use crate::census::AccountColumn;
use crate::census::AgentPlacement;
use crate::census::HierarchyEdges;
use crate::census::Machine;
use crate::census::MachineState;
use crate::census::account::ConfigHome;
use crate::config::AccountsConfig;
use crate::footer_colors::FooterColors;
use crate::summary;

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
            &[],
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
        "review",
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
