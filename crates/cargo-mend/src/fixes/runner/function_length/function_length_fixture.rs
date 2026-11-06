use tempfile::TempDir;

use super::FunctionLengthPlan;
use crate::fixes::imports_at_top;
use crate::fixes::prefer_module_import;
use crate::fixes::runner::FixScans;
use crate::fixes::runner::MendRunner;
use crate::reporting::AncestorReexport;
use crate::reporting::ModuleForm;
use crate::reporting::Report;
use crate::reporting::SubtreeReexportFixFact;
use crate::selection::Selection;
use crate::selection::SelectionScope;

pub(in crate::fixes::runner) struct FunctionLengthFixture(pub(super) TempDir);

impl FunctionLengthFixture {
    pub(in crate::fixes::runner) fn new(scenarios: &str, limit: usize) -> anyhow::Result<Self> {
        let fixture = Self(tempfile::tempdir()?);
        let root = fixture.0.path();
        std::fs::create_dir(root.join("src"))?;
        std::fs::write(
            root.join("clippy.toml"),
            format!("too-many-lines-threshold = {limit}\n"),
        )?;
        std::fs::write(root.join("rustfmt.toml"), "max_width = 60\n")?;
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"length_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        std::fs::write(
            root.join("src/lib.rs"),
            concat!(
                "mod unusually_long_module_name {\n",
                "    pub fn update(first: u32, second: u32, third: u32) {}\n",
                "}\nmod auxiliary { pub const VALUE: u32 = 1; }\nmod scenarios;\n",
            ),
        )?;
        std::fs::write(root.join("src/scenarios.rs"), scenarios)?;
        Ok(fixture)
    }

    pub(in crate::fixes::runner) fn selection(&self) -> anyhow::Result<Selection> {
        let root = std::fs::canonicalize(self.0.path())?;
        Ok(Selection {
            manifest_path:    root.join("Cargo.toml"),
            manifest_dir:     root.clone(),
            workspace_root:   root.clone(),
            target_directory: root.join("target"),
            analysis_root:    root.clone(),
            scope:            SelectionScope::Workspace,
            package_roots:    vec![root],
            packages:         Vec::new(),
        })
    }

    pub(super) fn plan(&self) -> anyhow::Result<(FunctionLengthPlan, Report)> {
        let selection = self.selection()?;
        let sources = crate::rust_syntax::ParsedSources::default();
        let module_imports = prefer_module_import::scan_selection(&selection, &sources)?;
        let moved = imports_at_top::scan_selection(&selection, &sources)?;
        let mut report = Report {
            findings: module_imports
                .findings
                .iter()
                .chain(&moved.findings)
                .cloned()
                .collect(),
            ..crate::reporting::Report::default()
        };
        let plan = MendRunner::combined_function_plan(
            FixScans {
                module_imports: Some(&module_imports),
                imports_at_top: Some(&moved),
                ..FixScans::default()
            },
            &sources,
            &selection.package_roots,
            &selection.workspace_root,
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        plan.filter_report(&mut report, &selection.analysis_root)?;
        Ok((plan, report))
    }
    pub(in crate::fixes::runner) fn subtree_report(&self) -> anyhow::Result<Report> {
        let selection = self.selection()?;
        let root = &selection.manifest_dir;
        let lib = root.join("src/lib.rs");
        std::fs::write(
            &lib,
            format!(
                "{}mod owner;\nmod source;\n",
                std::fs::read_to_string(&lib)?
            ),
        )?;
        std::fs::write(
            root.join("src/owner.rs"),
            "pub(crate) use crate::source::ping;\n",
        )?;
        std::fs::write(root.join("src/source.rs"), "pub(crate) fn ping() {}\n")?;
        let finding = serde_json::from_value(serde_json::json!({
            "severity": "warning", "diagnostic_code": "pub_use_outside_subtree",
            "path": "src/owner.rs", "line": 1, "column": 1, "highlight_len": 3,
            "source_line": "pub(crate) use crate::source::ping;", "item": "ping",
            "message": "re-export outside subtree", "suggestion": null,
            "fixability": "fix_pub_use_outside_subtree"
        }))?;
        let fact = SubtreeReexportFixFact {
            use_path:          "src/owner.rs".to_string(),
            use_line:          1,
            use_column:        1,
            exported_name:     "ping".to_string(),
            written_path:      "crate::source::ping".to_string(),
            owner_module:      vec!["owner".to_string()],
            owner_module_form: ModuleForm::File,
            source_module:     vec!["source".to_string()],
            target_scope:      Vec::new(),
            common_ancestor:   Vec::new(),
            crate_root_file:   "src/lib.rs".to_string(),
            ancestor_reexport: AncestorReexport::NotRequired,
        };
        let mut report = Report {
            findings: vec![finding],
            ..Report::default()
        };
        report.facts.subtree_reexport_fix_facts = vec![fact].into();
        Ok(report)
    }
}
