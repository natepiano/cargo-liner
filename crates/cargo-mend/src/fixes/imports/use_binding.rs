use syn::UseTree;

pub enum UseBinding {
    Named { name: String, path: String },
    Glob { path: String },
}

impl UseBinding {
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::Named { name, .. } => Some(name),
            Self::Glob { .. } => None,
        }
    }

    pub fn path(&self) -> &str {
        match self {
            Self::Named { path, .. } | Self::Glob { path } => path,
        }
    }

    pub fn binds_path_leaf(&self) -> bool {
        self.name()
            .is_some_and(|name| self.path().rsplit("::").next() == Some(name))
    }
}

pub fn collect_use_bindings(tree: &UseTree) -> Vec<UseBinding> {
    let mut bindings = Vec::new();
    collect(tree, &mut Vec::new(), &mut bindings);
    bindings
}

fn collect(tree: &UseTree, prefix: &mut Vec<String>, bindings: &mut Vec<UseBinding>) {
    match tree {
        UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect(&path.tree, prefix, bindings);
            prefix.pop();
        },
        UseTree::Name(name) => {
            let name = name.ident.to_string();
            if name == "self" {
                if let Some(bound_name) = prefix.last() {
                    bindings.push(UseBinding::Named {
                        name: bound_name.clone(),
                        path: prefix.join("::"),
                    });
                }
            } else {
                let mut path = prefix.clone();
                path.push(name.clone());
                bindings.push(UseBinding::Named {
                    name,
                    path: path.join("::"),
                });
            }
        },
        UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            let source_name = rename.ident.to_string();
            if source_name != "self" {
                path.push(source_name);
            }
            if !path.is_empty() {
                bindings.push(UseBinding::Named {
                    name: rename.rename.to_string(),
                    path: path.join("::"),
                });
            }
        },
        UseTree::Group(group) => {
            for item in &group.items {
                collect(item, prefix, bindings);
            }
        },
        UseTree::Glob(_) => bindings.push(UseBinding::Glob {
            path: prefix.join("::"),
        }),
    }
}
