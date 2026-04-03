//! Testing utility used to measure Rayon-based disk sizing.

use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use rayon::prelude::*;

#[derive(Parser)]
struct Args {
    /// Root directory used for testing, e.g. ~/rust
    root: String,
}

#[derive(Debug)]
struct EntrySize {
    name: String,
    bytes: u64,
}

fn main() -> Result<(), String> {
    let args = Args::parse();
    let root = expand_home(&args.root)?;
    if !root.is_dir() {
        return Err(format!("not a directory: {}", root.display()));
    }

    let started = Instant::now();
    let entries = top_level_entries(&root)?;
    let sizes: Vec<EntrySize> = entries
        .par_iter()
        .map(|entry| EntrySize {
            name: entry_name(entry),
            bytes: subtree_size_parallel(entry),
        })
        .collect();

    let total_bytes: u64 = sizes.iter().map(|entry| entry.bytes).sum();
    let elapsed = started.elapsed();

    println!("root: {}", root.display());
    println!("total_bytes: {total_bytes}");
    println!("elapsed_ms: {}", elapsed.as_millis());
    println!();
    for entry in sizes {
        println!("{:>16}  {}", format_bytes(entry.bytes), entry.name);
    }

    Ok(())
}

fn expand_home(raw: &str) -> Result<PathBuf, String> {
    if raw == "~" {
        return dirs::home_dir().ok_or_else(|| "home directory not found".to_string());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return dirs::home_dir()
            .map(|home| home.join(rest))
            .ok_or_else(|| "home directory not found".to_string());
    }
    Ok(PathBuf::from(raw))
}

fn top_level_entries(root: &Path) -> Result<Vec<PathBuf>, String> {
    fs::read_dir(root)
        .map_err(|err| format!("failed to read {}: {err}", root.display()))?
        .map(|entry| {
            entry
                .map(|entry| entry.path())
                .map_err(|err| format!("failed to read entry in {}: {err}", root.display()))
        })
        .collect()
}

fn entry_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().to_string(),
    )
}

fn subtree_size_parallel(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return 0;
    }
    if file_type.is_file() {
        return metadata.len();
    }
    if !file_type.is_dir() {
        return 0;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    let paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();

    paths
        .par_iter()
        .map(|child| subtree_size_parallel(child))
        .sum()
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut divisor = 1_u128;
    let mut unit_index = 0usize;
    while u128::from(bytes) / divisor >= 1024 && unit_index < UNITS.len() - 1 {
        divisor *= 1024;
        unit_index += 1;
    }
    let scaled = (u128::from(bytes) * 10 + divisor / 2) / divisor;
    format!("{}.{} {}", scaled / 10, scaled % 10, UNITS[unit_index])
}
