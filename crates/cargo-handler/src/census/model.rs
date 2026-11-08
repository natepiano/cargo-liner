//! Model names and reasoning effort recorded by each conversation.

use std::fmt::Formatter;

use serde::Deserialize;
use serde::Serialize;
use serde::de::IgnoredAny;
use serde::de::Visitor;
use serde_json::Value;

/// A model family, or a checked short name for an unknown family.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
pub(crate) enum ModelName {
    #[default]
    Unrecorded,
    Opus,
    Sonnet,
    Haiku,
    Fable,
    Astra,
    Sol,
    Other(ShortModelName),
}

/// A nonempty model word without version digits or punctuation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ShortModelName(String);

/// `MODEL_NAMES` supplies the same family words for record parsing and display.
const MODEL_NAMES: &[(&str, ModelName)] = &[
    ("opus", ModelName::Opus),
    ("sonnet", ModelName::Sonnet),
    ("haiku", ModelName::Haiku),
    ("fable", ModelName::Fable),
    ("astra", ModelName::Astra),
    ("sol", ModelName::Sol),
];

impl ModelName {
    pub(super) const fn is_unrecorded(&self) -> bool { matches!(self, Self::Unrecorded) }
    fn short(&self) -> &str {
        match self {
            Self::Unrecorded => "",
            Self::Other(name) => &name.0,
            family => MODEL_NAMES
                .iter()
                .find(|(_, value)| value == family)
                .map_or("", |(name, _)| *name),
        }
    }
}

impl From<&str> for ModelName {
    /// `ModelName::from` normalizes a model id at the record boundary.
    fn from(id: &str) -> Self {
        if id.is_empty() || id == "unrecorded" {
            return Self::Unrecorded;
        }
        let normalized = id.to_ascii_lowercase();
        let words: Vec<_> = normalized
            .split(|letter: char| !letter.is_ascii_alphabetic())
            .filter(|word| !word.is_empty() && *word != "v")
            .collect();
        if let Some((_, family)) = MODEL_NAMES.iter().find(|(name, _)| words.contains(name)) {
            return family.clone();
        }
        words.last().map_or(Self::Unrecorded, |word| {
            Self::Other(ShortModelName((*word).to_owned()))
        })
    }
}

impl From<String> for ModelName {
    fn from(id: String) -> Self { Self::from(id.as_str()) }
}

impl From<ModelName> for String {
    fn from(model: ModelName) -> Self {
        match model {
            ModelName::Unrecorded => "unrecorded".to_owned(),
            model => model.short().to_owned(),
        }
    }
}

/// The reasoning budget named by a conversation record.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ReasoningEffort {
    Low,
    Medium,
    High,
    XHigh,
    Max,
    #[default]
    #[serde(other)]
    Unrecorded,
}

impl ReasoningEffort {
    pub(super) fn is_unrecorded(effort: impl std::borrow::Borrow<Self>) -> bool {
        matches!(effort.borrow(), Self::Unrecorded)
    }
}

impl From<&str> for ReasoningEffort {
    fn from(word: &str) -> Self {
        match word {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" | "x-high" | "extra_high" => Self::XHigh,
            "max" => Self::Max,
            _ => Self::Unrecorded,
        }
    }
}

/// Model and effort from one conversation, with no inherited process values.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ConversationModel {
    pub(super) model:  ModelName,
    pub(super) effort: ReasoningEffort,
}

impl ConversationModel {
    /// `take_fields` converts JSON values at their record boundary.
    fn take_fields(&mut self, model: Option<&Value>, effort: Option<&Value>) {
        if let Some(value) = model {
            self.model = value
                .as_str()
                .map_or(ModelName::Unrecorded, ModelName::from);
        }
        if let Some(value) = effort {
            self.effort = value
                .as_str()
                .map_or(ReasoningEffort::Unrecorded, ReasoningEffort::from);
        }
    }

    pub(super) fn take_claude(&mut self, value: &Value) {
        let record = match value["type"].as_str() {
            Some("assistant") => &value["message"],
            Some("system") => value,
            _ => return,
        };
        self.take_fields(
            record.get("model"),
            record
                .get("effort")
                .or_else(|| record.get("effortLevel"))
                .or_else(|| record.get("reasoning_effort")),
        );
    }

    pub(super) fn take_codex(&mut self, value: &Value) {
        let payload = &value["payload"];
        let record = match value["type"].as_str() {
            Some("turn_context") => payload,
            Some("event_msg") if payload["type"] == "thread_settings_applied" => {
                &payload["thread_settings"]
            },
            _ => return,
        };
        self.take_fields(
            record.get("model"),
            record
                .get("effort")
                .or_else(|| record.get("reasoning_effort"))
                .or_else(|| record.get("model_reasoning_effort")),
        );
    }
}

/// `ModelRecordMetadata` skips message bodies at the JSON boundary.
struct ModelRecordMetadata(Value);

#[derive(Deserialize)]
enum ModelMetadataKey {
    #[serde(rename = "type")]
    Kind,
    #[serde(rename = "model")]
    Model,
    #[serde(rename = "effort")]
    Effort,
    #[serde(rename = "effortLevel")]
    EffortLevel,
    #[serde(rename = "reasoning_effort")]
    ReasoningEffort,
    #[serde(rename = "model_reasoning_effort")]
    ModelReasoningEffort,
    #[serde(rename = "message")]
    Message,
    #[serde(rename = "payload")]
    Payload,
    #[serde(rename = "thread_settings")]
    ThreadSettings,
    #[serde(other)]
    Ignore,
}

impl<'de> Deserialize<'de> for ModelRecordMetadata {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct MetadataVisitor;
        impl<'de> Visitor<'de> for MetadataVisitor {
            type Value = ModelRecordMetadata;

            fn expecting(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a conversation metadata record")
            }

            fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
                Ok(ModelRecordMetadata(Value::Null))
            }

            fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                self.visit_unit()
            }
            fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<Self::Value, E> {
                self.visit_unit()
            }

            fn visit_seq<S: serde::de::SeqAccess<'de>>(
                self,
                mut values: S,
            ) -> Result<Self::Value, S::Error> {
                while values.next_element::<IgnoredAny>()?.is_some() {}
                self.visit_unit()
            }

            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut map: M,
            ) -> Result<Self::Value, M::Error> {
                let mut fields = serde_json::Map::new();
                while let Some(key) = map.next_key::<ModelMetadataKey>()? {
                    let (name, value) = match key {
                        ModelMetadataKey::Kind => ("type", map.next_value()?),
                        ModelMetadataKey::Model => ("model", map.next_value()?),
                        ModelMetadataKey::Effort => ("effort", map.next_value()?),
                        ModelMetadataKey::EffortLevel => ("effortLevel", map.next_value()?),
                        ModelMetadataKey::ReasoningEffort => {
                            ("reasoning_effort", map.next_value()?)
                        },
                        ModelMetadataKey::ModelReasoningEffort => {
                            ("model_reasoning_effort", map.next_value()?)
                        },
                        ModelMetadataKey::Message => {
                            ("message", map.next_value::<ModelRecordMetadata>()?.0)
                        },
                        ModelMetadataKey::Payload => {
                            ("payload", map.next_value::<ModelRecordMetadata>()?.0)
                        },
                        ModelMetadataKey::ThreadSettings => (
                            "thread_settings",
                            map.next_value::<ModelRecordMetadata>()?.0,
                        ),
                        ModelMetadataKey::Ignore => {
                            map.next_value::<IgnoredAny>()?;
                            continue;
                        },
                    };
                    fields.insert(name.to_owned(), value);
                }
                Ok(ModelRecordMetadata(Value::Object(fields)))
            }
        }
        deserializer.deserialize_any(MetadataVisitor)
    }
}

/// `record_metadata` decodes settings without allocating unrelated transcript content.
pub(super) fn record_metadata(line: &[u8]) -> serde_json::Result<Value> {
    serde_json::from_slice::<ModelRecordMetadata>(line).map(|record| record.0)
}

#[cfg(test)]
#[expect(clippy::expect_used, reason = "fixture records must parse")]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;
    use crate::census::AgentRow;
    use crate::census::HierarchySource;
    use crate::census::SessionIdentity;
    use crate::census::StaffFileSetting;
    use crate::census::account::AccountLabels;
    use crate::census::account::ConfigHome;
    use crate::census::classify::ProcessEntry;
    use crate::census::classify::SessionRecord;
    use crate::census::hierarchy::StaffFileReader;
    use crate::census::scan::LocalScanner;

    const THREAD: &str = "01a18dbb-7e8f-7641-b3dd-bb2071ab9d7a";

    struct MetadataFixture {
        root:    TempDir,
        rollout: PathBuf,
    }

    impl MetadataFixture {
        fn new() -> Self {
            let root = TempDir::new().expect("metadata fixture");
            let sessions = root.path().join(".codex/sessions");
            std::fs::create_dir_all(&sessions).expect("Codex sessions");
            let rollout = sessions.join(format!("rollout-2026-11-07-{THREAD}.jsonl"));
            std::fs::write(
                &rollout,
                format!(
                    "{}\n{}\n",
                    json!({"type":"session_meta",
                "payload":{"id":THREAD,"cwd":"/work/fixture","timestamp":"2026-11-07T00:00:00Z"}}),
                    json!({"type":"turn_context","payload":{"model":"gpt-6.1-sol","effort":"high"}})
                ),
            )
            .expect("Codex metadata");
            let projects = root.path().join(".claude/projects/-work-fixture");
            std::fs::create_dir_all(&projects).expect("Claude projects");
            std::fs::write(
                projects.join("director.jsonl"),
                format!(
                    "{}\n",
                    json!({
                        "type":"assistant","message":{"model":"claude-opus-4-6","effort":"low"}
                    })
                ),
            )
            .expect("director metadata");
            std::fs::write(root.path().join("staff.json"), json!({"schema":2,"rows":[
                {"id":"boss","kind":"showrunner","name":"boss","parent":null,"current_session":"boss-session"},
                {"id":"unit","kind":"unit","name":"unit","parent":"boss","current_session":"director"},
                {"id":"worker","kind":"worker","name":"own worker","parent":"unit","current_session":THREAD}
            ]}).to_string()).expect("staff metadata");
            Self { root, rollout }
        }

        fn process(&self, pid: u32, arguments: &[&str]) -> ProcessEntry {
            ProcessEntry {
                pid,
                parent: Some(20),
                name: arguments[0].into(),
                arguments: arguments.iter().map(|word| (*word).into()).collect(),
                started: 100,
                directory: Some("/work/fixture".into()),
                claude_pid: None,
                config_home: ConfigHome::Named(self.root.path().join(".codex")),
            }
        }

        fn scan(&self, worker: ProcessEntry) -> Vec<AgentRow> {
            let sessions: Vec<SessionRecord> = serde_json::from_value(json!([
                {"pid":10,"sessionId":"boss-session","name":"boss"},
                {"pid":20,"sessionId":"director","name":"unit"}
            ]))
            .expect("session records");
            let mut processes = vec![self.process(10, &["claude"]), self.process(20, &["claude"])];
            processes[0].parent = Some(1);
            processes[1].parent = Some(1);
            processes.push(worker);
            let setting = StaffFileSetting::new(HierarchySource::File(
                self.root
                    .path()
                    .join("staff.json")
                    .to_string_lossy()
                    .into_owned(),
            ));
            let mut scanner = LocalScanner::new(AccountLabels::default())
                .with_staff_file(setting, StaffFileReader::default());
            scanner.scan_details_for_test(&processes, &sessions, self.root.path())
        }

        fn database(&self) {
            let db = rusqlite::Connection::open(self.root.path().join(".codex/state_6.sqlite"))
                .expect("thread database");
            db.execute_batch(
                "CREATE TABLE threads(id TEXT PRIMARY KEY,cwd TEXT,created_at_ms INTEGER,
                name TEXT,first_user_message TEXT,rollout_path TEXT,originator TEXT)",
            )
            .expect("thread schema");
            db.execute("INSERT INTO threads VALUES(?1,'/work/fixture',100000,'own worker','',?2,'codex-tui')",
                rusqlite::params![THREAD,self.rollout.to_string_lossy()]).expect("thread row");
        }
    }

    fn assert_worker_metadata(rows: &[AgentRow], pid: u32) {
        let worker = rows.iter().find(|row| row.pid == pid).expect("worker row");
        assert!(matches!(worker.session_id, SessionIdentity::Codex(_)));
        assert_eq!(worker.model, ModelName::Sol);
        assert_eq!(worker.effort, ReasoningEffort::High);
        let wire = serde_json::to_string(&rows).expect("probe rows");
        let received: Vec<AgentRow> = serde_json::from_str(&wire).expect("probe read");
        assert_eq!(received.as_slice(), rows);
        let mut older: Value = serde_json::from_str(&wire).expect("older probe");
        for row in older.as_array_mut().expect("probe rows") {
            row.as_object_mut().expect("probe row").remove("model");
            row.as_object_mut().expect("probe row").remove("effort");
            for child in row["children"].as_array_mut().expect("probe children") {
                child.as_object_mut().expect("probe child").remove("model");
                child.as_object_mut().expect("probe child").remove("effort");
            }
        }
        let older: Vec<AgentRow> = serde_json::from_value(older).expect("old probe compatibility");
        assert!(
            older.iter().all(|row| row.model == ModelName::Unrecorded
                && row.effort == ReasoningEffort::Unrecorded)
        );
        assert!(
            older
                .iter()
                .flat_map(|row| &row.children)
                .all(|row| row.model == ModelName::Unrecorded
                    && row.effort == ReasoningEffort::Unrecorded)
        );
    }

    #[test]
    fn subagents_keep_their_own_metadata_and_do_not_inherit_missing_values() {
        let fixture = MetadataFixture::new();
        let directory = fixture
            .root
            .path()
            .join(".claude/projects/-work-fixture/director/subagents");
        std::fs::create_dir_all(&directory).expect("subagent directory");
        for (id, message) in [
            ("own", json!({"model":"claude-haiku-4-5","effort":"medium"})),
            ("missing", json!({})),
        ] {
            std::fs::write(
                directory.join(format!("agent-{id}.meta.json")),
                json!({"agentType":"general-purpose","description":id}).to_string(),
            )
            .expect("subagent metadata");
            std::fs::write(
                directory.join(format!("agent-{id}.jsonl")),
                format!("{}\n", json!({"type":"assistant","message":message})),
            )
            .expect("subagent transcript");
        }
        let rows = fixture.scan(fixture.process(40, &["codex"]));
        let director = rows.iter().find(|row| row.pid == 20).expect("director row");
        let own = director
            .children
            .iter()
            .find(|row| row.name == "own")
            .expect("own subagent");
        assert_eq!(own.model, ModelName::Haiku);
        assert_eq!(own.effort, ReasoningEffort::Medium);
        let missing = director
            .children
            .iter()
            .find(|row| row.name == "missing")
            .expect("missing subagent");
        assert_eq!(missing.model, ModelName::Unrecorded);
        assert_eq!(missing.effort, ReasoningEffort::Unrecorded);
    }

    #[test]
    fn nested_claude_subagents_use_their_own_process_metadata_when_ids_collide() {
        let fixture = MetadataFixture::new();
        let projects = fixture.root.path().join(".claude/projects/-work-fixture");
        std::fs::write(
            projects.join("boss-session.jsonl"),
            format!(
                "{}\n",
                json!({"type":"assistant", "message":{"model":"claude-opus-4-6"}})
            ),
        )
        .expect("outer transcript");
        for (session, id, name, message) in [
            (
                "boss-session",
                "shared",
                "outer subagent",
                json!({"model":"claude-sonnet-4-5", "effort":"high"}),
            ),
            (
                "director",
                "own",
                "nested subagent",
                json!({"model":"claude-haiku-4-5", "effort":"medium"}),
            ),
            (
                "director",
                "shared",
                "nested shared subagent",
                json!({"model":"claude-haiku-4-5", "effort":"medium"}),
            ),
        ] {
            let directory = projects.join(session).join("subagents");
            std::fs::create_dir_all(&directory).expect("subagent directory");
            std::fs::write(
                directory.join(format!("agent-{id}.meta.json")),
                json!({"agentType":"general-purpose", "description":name}).to_string(),
            )
            .expect("subagent metadata");
            std::fs::write(
                directory.join(format!("agent-{id}.jsonl")),
                format!("{}\n", json!({"type":"assistant", "message":message})),
            )
            .expect("subagent transcript");
        }
        let mut outer = fixture.process(10, &["claude"]);
        outer.parent = Some(1);
        let mut nested = fixture.process(20, &["claude"]);
        nested.parent = Some(10);
        let sessions: Vec<SessionRecord> = serde_json::from_value(json!([
            {"pid":10,"sessionId":"boss-session","name":"outer"},
            {"pid":20,"sessionId":"director","name":"nested"}
        ]))
        .expect("Claude sessions");
        let setting = StaffFileSetting::new(HierarchySource::File(
            fixture
                .root
                .path()
                .join("staff.json")
                .to_string_lossy()
                .into_owned(),
        ));
        let mut scanner = LocalScanner::new(AccountLabels::default())
            .with_staff_file(setting, StaffFileReader::default());
        let rows = scanner.scan_details_for_test(&[outer, nested], &sessions, fixture.root.path());
        let outer = rows
            .iter()
            .find(|row| row.pid == 10)
            .expect("outer Claude row");
        for name in ["nested subagent", "nested shared subagent"] {
            let child = outer
                .children
                .iter()
                .find(|child| child.name == name)
                .expect("nested subagent row");
            assert_eq!(child.model, ModelName::Haiku, "{name}");
            assert_eq!(child.effort, ReasoningEffort::Medium, "{name}");
        }
        let outer_child = outer
            .children
            .iter()
            .find(|child| child.name == "outer subagent")
            .expect("outer subagent row");
        assert_eq!(outer_child.model, ModelName::Sonnet);
        assert_eq!(outer_child.effort, ReasoningEffort::High);
    }

    #[test]
    fn resumed_worker_keeps_its_own_metadata_on_the_probe_wire() {
        let fixture = MetadataFixture::new();
        fixture.database();
        let mut worker = fixture.process(40, &["codex", "resume", THREAD]);
        worker.parent = Some(1);
        assert_worker_metadata(&fixture.scan(worker), 40);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn open_staff_rollout_worker_keeps_its_metadata_on_the_probe_wire() {
        let fixture = MetadataFixture::new();
        let held = std::fs::File::open(&fixture.rollout).expect("held own rollout");
        let pid = std::process::id();
        let mut worker = fixture.process(pid, &["codex"]);
        worker.parent = Some(1);
        assert_worker_metadata(&fixture.scan(worker), pid);
        drop(held);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn hosted_thread_keeps_its_metadata_instead_of_the_directors_values() {
        let fixture = MetadataFixture::new();
        fixture.database();
        let held = std::fs::File::open(&fixture.rollout).expect("held own rollout");
        let rows = fixture.scan(fixture.process(std::process::id(), &["codex", "app-server"]));
        let director = rows.iter().find(|row| row.pid == 20).expect("director row");
        assert_eq!(director.model, ModelName::Opus);
        assert_eq!(director.effort, ReasoningEffort::Low);
        let child = director
            .children
            .iter()
            .find(|child| {
                matches!(
            &child.thread_id,crate::census::CodexThreadId::Recorded(id) if id==THREAD)
            })
            .expect("hosted thread");
        assert_eq!(child.model, ModelName::Sol);
        assert_eq!(child.effort, ReasoningEffort::High);
        let received: AgentRow =
            serde_json::from_str(&serde_json::to_string(director).expect("outgoing probe"))
                .expect("incoming probe");
        assert_eq!(received.children, director.children);
        drop(held);
    }

    #[test]
    fn recorded_model_ids_use_one_short_name_table() {
        for (id, short) in [
            ("claude-opus-4-6", "opus"),
            ("claude-sonnet-4-5-20250929", "sonnet"),
            ("claude-haiku-4-5", "haiku"),
            ("claude-fable-5", "fable"),
            ("gpt-6.1-astra", "astra"),
            ("openai/gpt-6.1-sol", "sol"),
        ] {
            assert_eq!(ModelName::from(id).short(), short, "{id}");
        }
    }

    #[test]
    fn unknown_model_ids_keep_the_last_word_without_version_digits() {
        assert_eq!(ModelName::from("vendor/new-comet-3.2").short(), "comet");
        assert_eq!(ModelName::from("vendor/nebula2").short(), "nebula");
    }

    #[test]
    fn reasoning_records_keep_each_supported_budget() {
        for (word, effort) in [
            ("low", ReasoningEffort::Low),
            ("medium", ReasoningEffort::Medium),
            ("high", ReasoningEffort::High),
            ("xhigh", ReasoningEffort::XHigh),
            ("max", ReasoningEffort::Max),
        ] {
            assert_eq!(ReasoningEffort::from(word), effort);
        }
    }
}
