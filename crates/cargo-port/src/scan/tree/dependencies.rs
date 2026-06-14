use super::AbsolutePath;
use super::CARGO_TOML;
use super::Component;
use super::HashMap;
use super::HashSet;
use super::Path;
use super::PathBuf;
use super::Table;
use super::Value;

pub(super) fn workspace_path_dependencies(workspace_path: &Path) -> HashMap<String, AbsolutePath> {
    let Some(table) = manifest_table(&workspace_path.join(CARGO_TOML)) else {
        return HashMap::new();
    };
    let Some(dependencies) = table
        .get("workspace")
        .and_then(Value::as_table)
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(Value::as_table)
    else {
        return HashMap::new();
    };

    dependencies
        .iter()
        .filter_map(|(name, value)| {
            dependency_path(value, workspace_path, name, &HashMap::new())
                .map(|path| (name.clone(), path))
        })
        .collect()
}

pub(super) fn package_path_dependencies(
    package_path: &Path,
    workspace_dependencies: &HashMap<String, AbsolutePath>,
) -> HashSet<AbsolutePath> {
    let Some(table) = manifest_table(&package_path.join(CARGO_TOML)) else {
        return HashSet::new();
    };
    let mut paths = HashSet::new();
    collect_dependency_paths(&table, package_path, workspace_dependencies, &mut paths);
    if let Some(targets) = table.get("target").and_then(Value::as_table) {
        for target in targets.values().filter_map(Value::as_table) {
            collect_dependency_paths(target, package_path, workspace_dependencies, &mut paths);
        }
    }
    paths
}

fn manifest_table(manifest_path: &Path) -> Option<Table> {
    let contents = std::fs::read_to_string(manifest_path).ok()?;
    contents.parse().ok()
}

fn collect_dependency_paths(
    table: &Table,
    package_path: &Path,
    workspace_dependencies: &HashMap<String, AbsolutePath>,
    paths: &mut HashSet<AbsolutePath>,
) {
    for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
        let Some(dependencies) = table.get(section).and_then(Value::as_table) else {
            continue;
        };
        for (name, value) in dependencies {
            if let Some(path) = dependency_path(value, package_path, name, workspace_dependencies) {
                paths.insert(path);
            }
        }
    }
}

fn dependency_path(
    value: &Value,
    base_path: &Path,
    name: &str,
    workspace_dependencies: &HashMap<String, AbsolutePath>,
) -> Option<AbsolutePath> {
    let table = value.as_table()?;
    if let Some(path) = table.get("path").and_then(Value::as_str) {
        return Some(resolve_dependency_path(path, base_path));
    }
    if table.get("workspace").and_then(Value::as_bool) == Some(true) {
        return workspace_dependencies.get(name).cloned();
    }
    None
}

fn resolve_dependency_path(path: &str, base_path: &Path) -> AbsolutePath {
    let raw_path = Path::new(path);
    let resolved = if raw_path.is_absolute() {
        raw_path.to_path_buf()
    } else {
        base_path.join(raw_path)
    };
    AbsolutePath::from(normalize_path_components(&resolved))
}

fn normalize_path_components(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {},
            Component::ParentDir => {
                normalized.pop();
            },
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            },
        }
    }
    normalized
}
