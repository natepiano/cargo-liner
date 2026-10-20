//! Pool rows follow live processes, current leases, and current review tasks.

#![expect(
    clippy::expect_used,
    reason = "fixture input and rendered rows must exist"
)]
#![expect(clippy::panic, reason = "a fixture probe must produce an answer")]

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use serde_json::Value;
use tier::RolloutRead;
use worker_dispatch::JobText;
use worker_dispatch::WorkerTask;

use super::hierarchy;
use super::hierarchy::HierarchyCache;
use super::hierarchy::HierarchyError;
use super::hierarchy::HierarchyRunner;
use super::hierarchy::ReviewerPools;
use super::hierarchy::ReviewerState;
use super::tier::RolloutStart;
use super::tier::Rollouts;
use super::*;
use crate::agent_cell::AgentTile;
use crate::agent_cell::{self};
use crate::footer_colors::FooterColors;

const REVIEW_PROMPT: &str = include_str!("fixtures/pooled_review_prompt.md");
const FIRST_TASK: &str = "Review the first worker change";
const SECOND_TASK: &str = "Review the second worker change";
const START: u64 = 1_792_180_800;

/// These variants are the worker-task contract before pool reviewers existed.
#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum LegacyWorkerTask {
    #[default]
    Unrecorded,
    RoleOnly,
    Task(String),
}

#[derive(serde::Deserialize)]
struct LegacyChild {
    name:        String,
    #[serde(default)]
    worker_task: LegacyWorkerTask,
}

#[derive(serde::Deserialize)]
struct LegacyRow {
    children: Vec<LegacyChild>,
}

#[derive(serde::Deserialize)]
struct LegacyReport {
    rows: Vec<LegacyRow>,
}

fn leased_rows() -> Vec<AgentRow> {
    let mut rows = rows();
    let mut reviewer = rows[2].children.remove(0);
    reviewer.worker_task = WorkerTask::Reviewer {
        name: "code3".into(),
        job:  JobText::Recorded(SECOND_TASK.into()),
    };
    // A carrier can already nest the live thread under a visible agent.
    rows.truncate(2);
    rows[1].children.push(reviewer);
    rows
}

fn reviewer_role(text: &str) -> String {
    let column = text
        .lines()
        .take_while(|line| !line.contains("code3"))
        .filter(|line| line.contains("session") && line.contains("status"))
        .last()
        .expect("the reviewer owner's tree header")
        .find("role")
        .expect("role column");
    text.lines()
        .find(|line| line.contains("code3"))
        .expect("reviewer row")
        .chars()
        .skip(column)
        .take_while(|character| !character.is_whitespace())
        .collect()
}

#[test]
fn reviewer_lease_draws_review_under_a_visible_agent_without_a_hierarchy_edge() {
    let text = drawn(leased_rows(), HierarchyEdges::default().into());
    assert!(text.contains(&format!("code3 · {SECOND_TASK}")), "{text}");
    assert_eq!(reviewer_role(&text), "review", "{text}");
}

#[test]
fn reviewer_schema_three_task_remains_readable_by_the_previous_receiver() {
    let payload = serde_json::to_vec(&serde_json::json!({
        "schema":3,"machine":"remote","rows":leased_rows(),"edges":[],
    }))
    .expect("current probe row encoding");
    let legacy: LegacyReport = serde_json::from_slice(&payload)
        .expect("the previous receiver accepts the current worker task");
    let child = &legacy.rows[1].children[0];
    assert_eq!(child.name, "old raw reviewer");
    match &child.worker_task {
        LegacyWorkerTask::Task(job) => assert_ne!(job, ""),
        LegacyWorkerTask::Unrecorded | LegacyWorkerTask::RoleOnly => {},
    }
    let MachineState::Answered(rows, snapshot) =
        RemoteHierarchyAnswers::default().apply("remote", probe::parse_answer(&payload))
    else {
        panic!("current receiver accepts the probe");
    };
    let text = drawn(rows, snapshot);
    assert!(text.contains(&format!("code3 · {SECOND_TASK}")), "{text}");
}

#[test]
fn reviewer_remote_busy_outcome_overrides_an_idle_link_after_discovery_fails() {
    let root = tempfile::tempdir().expect("pool root");
    pool(root.path(), serde_json::json!(std::process::id()), 100);
    let pools = ReviewerPools::read(root.path());
    let mut idle = hierarchy();
    idle.0[1].reviewer = ReviewerState::Idle;
    let first = serde_json::to_vec(&serde_json::json!({
        "schema":3,"machine":"remote","rows":leased_rows(),"edges":idle,
    }))
    .expect("idle hierarchy answer");
    let mut retained = RemoteHierarchyAnswers::default();
    let MachineState::Answered(rows, snapshot) =
        retained.apply("remote", probe::parse_answer(&first))
    else {
        panic!("idle answer");
    };
    assert!(!drawn(rows, snapshot).contains("code3"));
    let payload = serde_json::to_vec(&serde_json::json!({
        "schema":3,"machine":"remote","rows":leased_rows(),"edges":[],
        "reviewers":pools.outcomes(),
        "hierarchy_notice":HierarchyNotice::Failed(HierarchyError::TimedOut),
    }))
    .expect("busy outcome during failed discovery");
    let MachineState::Answered(rows, snapshot) =
        retained.apply("remote", probe::parse_answer(&payload))
    else {
        panic!("failed discovery retains its answer");
    };
    assert_eq!(
        snapshot.notice,
        HierarchyNotice::Failed(HierarchyError::TimedOut)
    );
    assert_eq!(snapshot.edges.0.len(), 3, "manager links remain in place");
    let text = drawn(rows, snapshot);
    assert!(text.contains(&format!("code3 · {SECOND_TASK}")), "{text}");
    assert_eq!(reviewer_role(&text), "review", "{text}");
}

struct LiveReview(Child);
impl LiveReview {
    fn start() -> Self {
        Self(
            Command::new("sh")
                .args(["-c", "read line"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("live review process"),
        )
    }
    fn pid(&self) -> u32 { self.0.id() }
    fn stop(&mut self) {
        self.0.kill().expect("stop review process");
        self.0.wait().expect("reap review process");
    }
}
impl Drop for LiveReview {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn rows() -> Vec<AgentRow> {
    serde_json::from_value(serde_json::json!([
        {"agent":"claude","session_id":"fixture-showrunner","name":"production","pid":10,"started":100,"directory":"/work/fixture","children":[]},
        {"agent":"claude","session_id":"fixture-unit","name":"handler-unit","pid":20,"started":101,"directory":"/work/fixture","children":[]},
        {"agent":"codex_thread_host","name":"app-server","pid":30,"started":102,"directory":"/work/fixture","children":[
            {"kind":"thread","depth":0,"pid":null,"thread_id":"review-thread","name":"old raw reviewer","started":103},
            {"kind":"thread","depth":0,"pid":null,"thread_id":"idle-thread","name":"idle pool reviewer","started":103},
            {"kind":"thread","depth":0,"pid":null,"thread_id":"template-thread","name":"pool template","started":103}
        ]}
    ])).expect("pool host and unit rows")
}

fn drawn(rows: Vec<AgentRow>, hierarchy: HierarchySnapshot) -> String {
    let state = MachineState::Answered(rows, hierarchy);
    let machines = [Machine {
        name:  "fixture",
        state: &state,
    }];
    let placement = AgentPlacement::from_machines(&machines);
    let cells = agent_cell::cell_order_placed(&placement);
    agent_cell::group_tiles(&placement, &cells, &HashMap::new(), START + 1000)
        .iter()
        .map(|tile| {
            let area = Rect::new(0, 0, 240, 20);
            let mut buffer = Buffer::empty(area);
            match tile {
                AgentTile::Agent(entry) => agent_cell::draw(
                    &mut buffer,
                    area,
                    entry,
                    &cells,
                    START + 1000,
                    AccountColumn::Hidden,
                    &FooterColors::default(),
                ),
                AgentTile::Unassigned(entry) => agent_cell::draw_unassigned(
                    &mut buffer,
                    area,
                    entry,
                    START + 1000,
                    AccountColumn::Hidden,
                    &FooterColors::default(),
                ),
            }
            (0..area.height)
                .map(|y| {
                    (0..area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn pool(root: &Path, pid: Value, modified: u64) {
    let directory = root.join("fixture-production");
    fs::create_dir_all(&directory).expect("pool directory");
    let path = directory.join("pool.json");
    let mut value: Value =
        serde_json::from_str(include_str!("fixtures/reviewer_pool.json")).expect("pool shape");
    value["reviewers"]["code3"]["busy_pid"] = pid;
    fs::write(&path, value.to_string()).expect("pool document");
    fs::File::options()
        .write(true)
        .open(path)
        .expect("pool file")
        .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(modified))
        .expect("different pool mtime");
}

fn hierarchy() -> HierarchyEdges {
    hierarchy::parse(include_bytes!("fixtures/reviewer_hierarchy.json"))
        .expect("production tree edge shape")
}

#[test]
fn reviewer_busy_pool_edge_draws_under_its_unit_while_idle_and_template_threads_draw_nothing() {
    let root = tempfile::tempdir().expect("pool root");
    pool(root.path(), serde_json::json!(std::process::id()), 100);
    let pools = ReviewerPools::read(root.path());
    let mut edges = hierarchy();
    pools.refresh(&mut edges);
    let mut rows = rows();
    ReviewerPools::apply_outcomes(&mut rows, &pools.outcomes());
    ReviewerPools::apply(&mut rows, &edges);
    let text = drawn(rows, edges.into());
    let unit = text
        .lines()
        .position(|line| line.contains("handler-unit"))
        .expect("unit row");
    let review = text
        .lines()
        .position(|line| line.contains("code3"))
        .expect("named reviewer row");
    assert!(review > unit, "{text}");
    let line = text.lines().nth(review).expect("review row");
    assert!(
        line.split_whitespace().any(|word| word == "review"),
        "{text}"
    );
    assert!(!text.contains("idle pool reviewer"), "{text}");
    assert!(!text.contains("pool template"), "{text}");
    assert!(!text.contains("old raw reviewer"), "{text}");
}

#[test]
fn reviewer_process_exit_hides_the_row_without_a_pool_file_rewrite() {
    let mut process = LiveReview::start();
    let root = tempfile::tempdir().expect("pool root");
    pool(root.path(), serde_json::json!(process.pid()), 100);
    let pools = ReviewerPools::read(root.path());
    let mut edges = hierarchy();
    pools.refresh(&mut edges);
    let mut before = rows();
    ReviewerPools::apply(&mut before, &edges);
    assert!(drawn(before, edges.clone().into()).contains("code3"));
    process.stop();
    pools.refresh(&mut edges);
    let mut after = rows();
    ReviewerPools::apply(&mut after, &edges);
    let text = drawn(after, edges.into());
    assert!(
        !text.contains("code3") && !text.contains("old raw reviewer"),
        "{text}"
    );
}

struct TreeAnswer;
impl HierarchyRunner for TreeAnswer {
    fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
        Ok(hierarchy())
    }
}

#[test]
fn reviewer_pool_mtime_changes_busy_idle_busy_with_the_same_director_and_app_server() {
    let root = tempfile::tempdir().expect("pool root");
    let mut cache = HierarchyCache::with_pool_root(root.path().to_owned());
    let source = HierarchySource::Command("fixture-production-tree".into());
    let now = Instant::now();
    for (pid, modified, visible) in [
        (serde_json::json!(std::process::id()), 100, true),
        (Value::Null, 200, false),
        (serde_json::json!(std::process::id()), 300, true),
    ] {
        pool(root.path(), pid, modified);
        let mut rows = rows();
        let snapshot = cache.snapshot(source.clone(), &rows, &TreeAnswer, now);
        ReviewerPools::apply(&mut rows, &snapshot.edges);
        let text = drawn(rows, snapshot);
        assert_eq!(text.contains("code3"), visible, "{text}");
        assert!(text.contains("handler-unit"), "{text}");
    }
}

fn task(text: &str, second: u32) -> String {
    format!(
        "{}\n",
        serde_json::json!({"timestamp":format!("2026-10-16T20:00:{second:02}Z"),"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}})
    )
}

fn head() -> String {
    format!(
        "{}\n{}",
        serde_json::json!({"type":"session_meta","payload":{"timestamp":"2026-10-16T20:00:00Z","cwd":"/work/fixture"}}),
        task(FIRST_TASK, 1)
    )
}

fn activity() -> &'static str {
    "{\"type\":\"event_msg\",\"payload\":{\"type\":\"thread_settings_applied\",\"thread_settings\":{\"service_tier\":\"default\"}}}\n{\"type\":\"event_msg\",\"payload\":{\"type\":\"task_started\"}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call\",\"name\":\"exec_command\",\"arguments\":\"{\\\"cmd\\\":\\\"rg reviewer src\\\"}\"}}\n"
}

fn current_job_row(read: &RolloutRead, lease: u64) -> String {
    let mut rows = rows();
    let mut edges = hierarchy();
    edges.0[1].reviewer = ReviewerState::Busy {
        name:    "code3".into(),
        pid:     std::process::id(),
        started: lease,
    };
    rows[2].children[0].worker_task = read.reviewer_task("code3", lease);
    rows[2].children[0].activity = read.activity();
    ReviewerPools::apply(&mut rows, &edges);
    drawn(rows, edges.into())
}

#[test]
fn reviewer_cold_rollout_draws_the_second_job_before_multiple_tail_growth_steps() {
    let directory = tempfile::tempdir().expect("rollout directory");
    let path = directory.path().join("review.jsonl");
    let neutral = format!(
        "{{\"type\":\"progress\",\"padding\":\"{}\"}}\n",
        "x".repeat(32 * 1024)
    );
    let content = format!(
        "{}{}{}{}",
        head(),
        task(SECOND_TASK, 20),
        neutral.repeat(12),
        activity()
    );
    fs::write(&path, &content).expect("two-job rollout");
    let mut rollouts = Rollouts::default();
    let read = rollouts.read(&path);
    assert_eq!(read.first_prompt.text(), FIRST_TASK);
    assert!(read.bytes_read() <= u64::try_from(content.len()).expect("file byte count"));
    let text = current_job_row(&read, START * 1000);
    assert!(text.contains(&format!("code3 · {SECOND_TASK}")), "{text}");
    assert!(!text.contains(FIRST_TASK), "{text}");
    assert!(text.contains("reading"), "{text}");
}

#[test]
fn reviewer_appended_job_replaces_the_previous_job_without_reading_old_bytes() {
    let directory = tempfile::tempdir().expect("rollout directory");
    let path = directory.path().join("review.jsonl");
    fs::write(&path, format!("{}{}", head(), activity())).expect("first job");
    let mut rollouts = Rollouts::default();
    let first = rollouts.read(&path);
    assert!(current_job_row(&first, START * 1000).contains(FIRST_TASK));
    let append = format!("{}{}", task(SECOND_TASK, 20), activity());
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append file")
        .write_all(append.as_bytes())
        .expect("second job");
    let second = rollouts.read(&path);
    assert_eq!(
        second.bytes_read(),
        u64::try_from(append.len()).expect("byte count")
    );
    let text = current_job_row(&second, START * 1000);
    assert!(text.contains(&format!("code3 · {SECOND_TASK}")), "{text}");
    assert!(!text.contains(FIRST_TASK), "{text}");
}

#[test]
fn reviewer_new_lease_draws_role_only_until_its_prompt_arrives() {
    let directory = tempfile::tempdir().expect("rollout directory");
    let path = directory.path().join("review.jsonl");
    fs::write(&path, format!("{}{}", head(), activity())).expect("old review");
    let mut rollouts = Rollouts::default();
    let previous = rollouts.read(&path);
    let lease = (START + 1) * 1000 + 500;
    let text = current_job_row(&previous, lease);
    let line = text
        .lines()
        .find(|line| line.contains("code3"))
        .expect("reviewer role-only row");
    assert!(!line.contains('·'), "{text}");
    assert!(!text.contains(FIRST_TASK), "{text}");
    let append = format!("{}{}", task(SECOND_TASK, 20), activity());
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append file")
        .write_all(append.as_bytes())
        .expect("new review");
    let next = rollouts.read(&path);
    assert!(current_job_row(&next, lease).contains(SECOND_TASK));
}

#[test]
fn reviewer_real_pool_opening_draws_the_review_job_instead_of_the_memory_or_preamble() {
    let directory = tempfile::tempdir().expect("rollout directory");
    let path = directory.path().join("review.jsonl");
    fs::write(
        &path,
        format!("{}{}{}", head(), task(REVIEW_PROMPT, 20), activity()),
    )
    .expect("real review prompt");
    let read = Rollouts::default().read(&path);
    let text = current_job_row(&read, START * 1000);
    assert!(
        text.contains("code3 · Accounts table in the summary"),
        "{text}"
    );
    assert!(
        !text.contains("review memory") && !text.contains("You are a pooled"),
        "{text}"
    );
}

#[test]
fn reviewer_rollout_start_records_session_time_and_keeps_missing_time_unavailable() {
    let directory = tempfile::tempdir().expect("rollout directory");
    let recorded = directory.path().join("recorded.jsonl");
    fs::write(&recorded, format!("{}{}", head(), activity())).expect("recorded start");
    let missing = directory.path().join("missing.jsonl");
    fs::write(&missing, task(SECOND_TASK, 20)).expect("no session metadata");
    let mut rollouts = Rollouts::default();
    assert_eq!(
        rollouts.read(&recorded).began,
        RolloutStart::Recorded(START)
    );
    assert_eq!(rollouts.read(&missing).began, RolloutStart::Unavailable);
}

struct FailedTree;
impl HierarchyRunner for FailedTree {
    fn run(&self, _: &str, _: Duration) -> Result<HierarchyEdges, HierarchyError> {
        Err(HierarchyError::TimedOut)
    }
}

#[test]
fn reviewer_process_exit_still_hides_the_row_when_discovery_keeps_old_links() {
    let mut process = LiveReview::start();
    let root = tempfile::tempdir().expect("pool root");
    pool(root.path(), serde_json::json!(process.pid()), 100);
    let mut cache = HierarchyCache::with_pool_root(root.path().to_owned());
    let source = HierarchySource::Command("fixture-production-tree".into());
    let now = Instant::now();
    let first = cache.snapshot(source.clone(), &rows(), &TreeAnswer, now);
    let mut before = rows();
    ReviewerPools::apply(&mut before, &first.edges);
    assert!(drawn(before, first).contains("code3"));
    process.stop();
    let failure = cache.snapshot(source, &rows(), &FailedTree, now + Duration::from_secs(31));
    assert_eq!(
        failure.notice,
        HierarchyNotice::Failed(HierarchyError::TimedOut)
    );
    assert_eq!(
        failure.edges.0.len(),
        3,
        "management links survive the failed command"
    );
    let mut after = rows();
    ReviewerPools::apply(&mut after, &failure.edges);
    let text = drawn(after, failure);
    assert!(text.contains("handler-unit"), "{text}");
    assert!(
        !text.contains("code3") && !text.contains("old raw reviewer"),
        "{text}"
    );
}

#[test]
fn reviewer_remote_process_exit_reaches_the_receiver_even_when_hierarchy_discovery_fails() {
    let mut process = LiveReview::start();
    let root = tempfile::tempdir().expect("pool root");
    pool(root.path(), serde_json::json!(process.pid()), 100);
    let pools = ReviewerPools::read(root.path());
    let mut edges = hierarchy();
    pools.refresh(&mut edges);
    let mut source_rows = rows();
    ReviewerPools::apply(&mut source_rows, &edges);
    // The carrier can retain its thread after the review process exits.
    let report = |notice, links: HierarchyEdges| {
        serde_json::to_vec(&serde_json::json!({
            "schema":3,"machine":"remote","rows":source_rows,"edges":links,
            "reviewers":pools.outcomes(),"hierarchy_notice":notice
        }))
        .expect("source-side probe report")
    };
    let first = report(HierarchyNotice::Clear, edges.clone());
    let mut retained = RemoteHierarchyAnswers::default();
    let state = retained.apply("remote", probe::parse_answer(&first));
    let MachineState::Answered(first_rows, first_hierarchy) = state else {
        panic!("remote answer");
    };
    assert!(drawn(first_rows, first_hierarchy).contains("code3"));
    process.stop();
    for notice in [
        HierarchyNotice::Clear,
        HierarchyNotice::Failed(HierarchyError::TimedOut),
    ] {
        // Each failure starts with busy links, not a prior idle answer.
        let mut retained = RemoteHierarchyAnswers::default();
        let previous = retained.apply("remote", probe::parse_answer(&first));
        let MachineState::Answered(previous_rows, previous_hierarchy) = previous else {
            panic!("remote busy answer");
        };
        assert!(drawn(previous_rows, previous_hierarchy).contains("code3"));
        pools.refresh(&mut edges);
        let payload = report(notice, edges.clone());
        let state = retained.apply("remote", probe::parse_answer(&payload));
        let MachineState::Answered(remote_rows, snapshot) = state else {
            panic!("remote answer");
        };
        assert_eq!(snapshot.notice, notice);
        assert_eq!(
            snapshot.edges.0.len(),
            3,
            "the receiver retains manager links"
        );
        let text = drawn(remote_rows, snapshot);
        assert!(text.contains("handler-unit"), "{text}");
        assert!(
            !text.contains("code3") && !text.contains("old raw reviewer"),
            "{text}"
        );
    }
}

#[test]
fn reviewer_two_live_leases_draw_separate_review_rows_under_one_unit() {
    let first = LiveReview::start();
    let second = LiveReview::start();
    let root = tempfile::tempdir().expect("pool root");
    pool(root.path(), serde_json::json!(first.pid()), 100);
    let path = root.path().join("fixture-production/pool.json");
    let mut document: Value =
        serde_json::from_slice(&fs::read(&path).expect("pool file")).expect("pool document");
    document["reviewers"]["design1"]["busy_pid"] = serde_json::json!(second.pid());
    fs::write(path, document.to_string()).expect("second live lease");
    let pools = ReviewerPools::read(root.path());
    let mut edges = hierarchy();
    pools.refresh(&mut edges);
    let mut rows = rows();
    ReviewerPools::apply_outcomes(&mut rows, &pools.outcomes());
    ReviewerPools::apply(&mut rows, &edges);
    let text = drawn(rows, edges.into());
    assert_eq!(text.matches("code3").count(), 1, "{text}");
    assert_eq!(text.matches("design1").count(), 1, "{text}");
    for name in ["code3", "design1"] {
        let line = text
            .lines()
            .find(|line| line.contains(name))
            .expect("review row");
        assert!(
            line.split_whitespace().any(|word| word == "review"),
            "{text}"
        );
    }
}

#[test]
fn reviewer_pool_preamble_without_job_text_draws_only_the_reviewer_name() {
    let directory = tempfile::tempdir().expect("rollout directory");
    let path = directory.path().join("review.jsonl");
    let opening = "You are a pooled reviewer for handler-enh-unit, phase 8. Edit nothing.";
    fs::write(
        &path,
        format!("{}{}{}", head(), task(opening, 20), activity()),
    )
    .expect("review prompt without a job");
    let read = Rollouts::default().read(&path);
    let text = current_job_row(&read, START * 1000);
    let line = text
        .lines()
        .find(|line| line.contains("code3"))
        .expect("reviewer row");
    assert!(!line.contains('·'), "{text}");
    assert!(
        line.split_whitespace().any(|word| word == "review"),
        "{text}"
    );
    assert!(!text.contains("You are a pooled"), "{text}");
}
