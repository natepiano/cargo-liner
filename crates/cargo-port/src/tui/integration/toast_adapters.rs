use tui_pane::TrackedItemKey;

use crate::ci::OwnerRepo;
use crate::project::AbsolutePath;

pub(crate) fn path_key(path: &AbsolutePath) -> TrackedItemKey {
    TrackedItemKey::new(path.to_string())
}

pub(crate) fn owner_repo_key(repo: &OwnerRepo) -> TrackedItemKey {
    TrackedItemKey::new(repo.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_key_uses_cargo_port_absolute_path_string() {
        let path = AbsolutePath::from("/tmp/cargo-port");

        assert_eq!(path_key(&path).as_str(), "/tmp/cargo-port");
    }

    #[test]
    fn owner_repo_key_uses_cargo_port_owner_repo_string() {
        let repo = OwnerRepo::new("natepiano", "cargo-port");

        assert_eq!(owner_repo_key(&repo).as_str(), "natepiano/cargo-port");
    }
}
