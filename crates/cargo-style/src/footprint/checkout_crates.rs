//! Find crate roots and library names in the repository checkout.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::module_sites::CrateLibrary;
use crate::module_sites::NoLibrary;

enum CrateRootFiles {
    Crate(CrateLibrary),
    NoCrate,
}

pub(super) enum CrateRoot {
    Found {
        folder:  PathBuf,
        library: CrateLibrary,
    },
    Absent,
}

pub(super) struct CheckoutCrates<'a> {
    checkout: &'a Path,
    folders:  HashMap<PathBuf, CrateRootFiles>,
}

impl<'a> CheckoutCrates<'a> {
    pub(super) fn new(checkout: &'a Path) -> Self {
        Self {
            checkout,
            folders: HashMap::new(),
        }
    }

    pub(super) fn root_for(&mut self, path: &str) -> CrateRoot {
        for folder in Path::new(path).ancestors().skip(1) {
            let files = self
                .folders
                .entry(folder.to_owned())
                .or_insert_with(|| read_root_files(self.checkout, folder));
            if let CrateRootFiles::Crate(library) = files {
                return CrateRoot::Found {
                    folder:  folder.to_owned(),
                    library: library.clone(),
                };
            }
        }
        CrateRoot::Absent
    }
}

fn read_root_files(checkout: &Path, folder: &Path) -> CrateRootFiles {
    let directory = checkout.join(folder);
    if directory.join("lib.rs").is_file() {
        let manifest = checkout
            .join(folder.parent().unwrap_or_else(|| Path::new("")))
            .join("Cargo.toml");
        return CrateRootFiles::Crate(crate_library(&manifest));
    }
    if directory.join("main.rs").is_file() {
        return CrateRootFiles::Crate(CrateLibrary::None(NoLibrary::None));
    }
    CrateRootFiles::NoCrate
}

fn crate_library(manifest: &Path) -> CrateLibrary {
    if manifest.is_file()
        && let Ok(text) = fs::read_to_string(manifest)
        && let Ok(data) = toml::from_str::<toml::Table>(&text)
    {
        for table in ["lib", "package"] {
            if let Some(name) = data
                .get(table)
                .and_then(|table| table.get("name"))
                .and_then(toml::Value::as_str)
            {
                return CrateLibrary::Named {
                    name: name.replace('-', "_"),
                };
            }
        }
    }
    CrateLibrary::None(NoLibrary::None)
}
