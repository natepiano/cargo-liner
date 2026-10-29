//! Content-based file identity.

use std::fs;
use std::io;
use std::path::Path;

use sha2::Digest as _;

/// Fingerprint for a single file. Equality is the SHA-256 of the file's
/// bytes. Inode is deliberately omitted — editor atomic-save workflows
/// rotate inodes on every save even when bytes are identical.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileContentHash {
    /// SHA-256 digest of the file bytes.
    pub content_hash: [u8; 32],
}

impl FileContentHash {
    /// Read `path` and compute its stamp. SHA-256 of the bytes is the
    /// authoritative identity.
    ///
    /// # Errors
    ///
    /// Returns the I/O error if reading `path` fails.
    pub fn from_path(path: &Path) -> io::Result<Self> {
        let bytes = fs::read(path)?;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&bytes);
        let digest = hasher.finalize();
        let mut content_hash = [0_u8; 32];
        content_hash.copy_from_slice(&digest);
        Ok(Self { content_hash })
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn write_file(path: &Path, body: &[u8]) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn equality_is_content_hash_only() {
        let a = FileContentHash {
            content_hash: [7; 32],
        };
        let b = FileContentHash {
            content_hash: [7; 32],
        };
        assert_eq!(a, b, "same hash → equal");

        let c = FileContentHash {
            content_hash: [8; 32],
        };
        assert_ne!(a, c, "different hash → unequal");
    }

    #[test]
    fn same_bytes_produce_same_content_hash() {
        let tmp = TempDir::new().unwrap();
        let first_path = tmp.path().join("a.toml");
        let second_path = tmp.path().join("b.toml");
        write_file(&first_path, b"payload");
        write_file(&second_path, b"payload");

        let first = FileContentHash::from_path(&first_path).unwrap();
        let second = FileContentHash::from_path(&second_path).unwrap();
        assert_eq!(
            first.content_hash, second.content_hash,
            "identical bytes hash identically"
        );
    }

    #[test]
    fn identical_bytes_written_via_rename_stay_equal() {
        // `fs::rename` moves `tmp_path` to `path` to simulate an editor atomic save:
        // the inode changes but the bytes do not, so `FileContentHash::from_path` must
        // produce equal `first` and `second` fingerprints without invalidation.
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("file.toml");
        write_file(&path, b"payload");
        let first = FileContentHash::from_path(&path).unwrap();

        let tmp_path = tmp.path().join("file.toml.new");
        write_file(&tmp_path, b"payload");
        fs::rename(&tmp_path, &path).unwrap();

        let second = FileContentHash::from_path(&path).unwrap();
        assert_eq!(
            first, second,
            "same bytes → same stamp, even after rename-save"
        );
    }
}
