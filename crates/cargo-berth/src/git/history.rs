//! Parent links read from commit objects without a revision-walk child.

use std::collections::HashMap;
use std::path::Path;

use super::error::GitError;
use super::object::CommitObjectResolution;
use super::read;
use super::read::GitReadRequirement;
use super::read::RepositoryRead;
use crate::ids::GitObjectId;
#[cfg(test)]
use crate::test_work;
#[cfg(test)]
use crate::test_work::Work;

/// A resolved tip and all of its reachable parent links.
pub(super) struct CommitHistory {
    pub(super) target:  GitObjectId,
    pub(super) parents: HashMap<GitObjectId, Vec<GitObjectId>>,
}

/// Exact objects and ordinary refs use the same compatibility boundary as other reads.
pub(super) fn read(
    root: &Path,
    expression: &str,
) -> Result<RepositoryRead<CommitHistory>, GitError> {
    #[cfg(test)]
    test_work::record(Work::HistoryRead);
    let target = match read::commit(root, expression)? {
        RepositoryRead::InProcess(CommitObjectResolution::Resolved(target)) => target,
        RepositoryRead::InProcess(_) => {
            return Ok(RepositoryRead::GitRequired(
                GitReadRequirement::RevisionExpression,
            ));
        },
        RepositoryRead::GitRequired(reason) => return Ok(RepositoryRead::GitRequired(reason)),
    };
    let repository = match read::open(root)? {
        RepositoryRead::InProcess(repository) => repository,
        RepositoryRead::GitRequired(reason) => return Ok(RepositoryRead::GitRequired(reason)),
    };
    if repository
        .is_shallow()
        .map_err(|error| read::failure("read shallow boundary", error))?
        || repository.common_dir().join("info/grafts").exists()
    {
        return Ok(RepositoryRead::GitRequired(
            GitReadRequirement::HistoryOverrides,
        ));
    }
    let mut parents = HashMap::new();
    let mut pending = vec![target.clone()];
    while let Some(id) = pending.pop() {
        if parents.contains_key(&id) {
            continue;
        }
        let object_id = gix::ObjectId::from_hex(id.to_string().as_bytes())
            .map_err(|error| read::failure("read history object id", error))?;
        let commit = repository
            .find_commit(object_id)
            .map_err(|error| read::failure("read history commit", error))?;
        let links = commit
            .decode()
            .map_err(|error| read::failure("decode history commit", error))?
            .parents()
            .map(|id| id.to_string().parse().map_err(GitError::InvalidObjectId))
            .collect::<Result<Vec<GitObjectId>, _>>()?;
        pending.extend(links.iter().cloned());
        parents.insert(id, links);
    }
    Ok(RepositoryRead::InProcess(CommitHistory { target, parents }))
}

#[cfg(test)]
mod tests {
    use super::read;
    use crate::git::command;
    use crate::git::fixture::FixtureResult;
    use crate::git::fixture::PatchEquivalenceFixture;
    use crate::git::read::GitReadRequirement;
    use crate::git::read::RepositoryRead;

    #[test]
    fn object_history_preserves_merge_parent_order_and_replacement_links() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.git(&["checkout", "-b", "side"])?;
        fixture.write("side", "side\n")?;
        let side = fixture.commit("side")?;
        fixture.git(&["checkout", "main"])?;
        fixture.write("main", "main\n")?;
        fixture.commit("main")?;
        fixture.git(&["merge", "--no-ff", "--quiet", "-m", "merge", "side"])?;
        assert_git_history(&fixture)?;
        // Replacement objects alter ancestry without changing the ref's object ID.
        fixture.git(&["replace", "--graft", "HEAD", &side.to_string()])?;
        assert!(matches!(
            read(fixture.root(), "HEAD")?,
            RepositoryRead::GitRequired(GitReadRequirement::HistoryOverrides)
        ));
        Ok(())
    }

    fn assert_git_history(fixture: &PatchEquivalenceFixture) -> FixtureResult {
        let RepositoryRead::InProcess(history) = read(fixture.root(), "HEAD")? else {
            return Err("ordinary commit history should need no Git child".into());
        };
        let output = command::git_output(fixture.root(), ["rev-list", "--parents", "HEAD"])?;
        assert!(output.status.success(), "{output:?}");
        let text = String::from_utf8(output.stdout)?;
        let mut expected = std::collections::HashMap::new();
        for line in text.lines() {
            let mut fields = line.split_whitespace();
            let id = fields.next().ok_or("missing history object")?.parse()?;
            let parents = fields.map(str::parse).collect::<Result<Vec<_>, _>>()?;
            expected.insert(id, parents);
        }
        assert_eq!(history.parents, expected);
        Ok(())
    }

    #[test]
    fn object_history_delegates_shallow_boundaries_and_legacy_grafts() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        fixture.write("later", "later\n")?;
        let later = fixture.commit("later")?;
        std::fs::write(fixture.root().join(".git/shallow"), format!("{later}\n"))?;
        assert!(matches!(
            read(fixture.root(), "HEAD")?,
            RepositoryRead::GitRequired(GitReadRequirement::HistoryOverrides)
        ));
        std::fs::remove_file(fixture.root().join(".git/shallow"))?;
        std::fs::write(
            fixture.root().join(".git/info/grafts"),
            format!("{later}\n"),
        )?;
        assert!(matches!(
            read(fixture.root(), "HEAD")?,
            RepositoryRead::GitRequired(GitReadRequirement::HistoryOverrides)
        ));
        Ok(())
    }
}
