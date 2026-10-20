//! Scan details carry each subagent transcript's own activity to its drawn row.

#![expect(
    clippy::expect_used,
    reason = "isolated transcript fixture files must exist"
)]

use std::io::Write;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

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
