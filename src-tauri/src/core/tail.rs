//! Helpers for append-only JSONL logs: newest file lookup and bounded reads from the end.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Tail sizes tried in order until the wanted line is found.
const TAIL_STEPS: [u64; 3] = [128 * 1024, 2 * 1024 * 1024, 24 * 1024 * 1024];

/// Newest file (by mtime) below `root` whose name ends with one of `suffixes`.
pub fn newest_file(
    root: &Path,
    suffixes: &[&str],
    max_depth: usize,
) -> Option<(PathBuf, SystemTime)> {
    let mut best: Option<(PathBuf, SystemTime)> = None;
    visit(root, suffixes, max_depth, &mut best);
    best
}

fn visit(dir: &Path, suffixes: &[&str], depth: usize, best: &mut Option<(PathBuf, SystemTime)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if depth > 0 {
                visit(&path, suffixes, depth - 1, best);
            }
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if !suffixes.iter().any(|suffix| name.ends_with(suffix)) {
            continue;
        }
        let Ok(modified) = entry.metadata().and_then(|meta| meta.modified()) else {
            continue;
        };
        if best.as_ref().is_none_or(|(_, at)| modified > *at) {
            *best = Some((path, modified));
        }
    }
}

/// Reads at most `max_bytes` from the end of the file and returns its lines in
/// file order. The second value is true when the read started at offset 0.
/// A line cut in half by the offset is dropped.
pub fn read_tail(path: &Path, max_bytes: u64) -> io::Result<(Vec<String>, bool)> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let start = size.saturating_sub(max_bytes);
    file.seek(SeekFrom::Start(start))?;
    let mut buffer = Vec::new();
    file.take(max_bytes).read_to_end(&mut buffer)?;

    let text = String::from_utf8_lossy(&buffer);
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r').to_string())
        .collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    lines.retain(|line| !line.trim().is_empty());
    Ok((lines, start == 0))
}

/// Walks the file backwards (newest line first) and returns the first value
/// `pick` produces. Lines that are still being written simply fail to parse
/// inside `pick` and are skipped.
pub fn find_last<T>(path: &Path, pick: impl Fn(&str) -> Option<T>) -> Option<T> {
    for max_bytes in TAIL_STEPS {
        let (lines, reached_start) = read_tail(path, max_bytes).ok()?;
        if let Some(found) = lines.iter().rev().find_map(|line| pick(line)) {
            return Some(found);
        }
        if reached_start {
            break;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn tail_drops_partial_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        fs::write(&path, "first line\r\nsecond line\nthird\n").unwrap();

        let (all, from_start) = read_tail(&path, 1024).unwrap();
        assert!(from_start);
        assert_eq!(all, vec!["first line", "second line", "third"]);

        let (tail, from_start) = read_tail(&path, 14).unwrap();
        assert!(!from_start);
        assert_eq!(tail, vec!["third"]);
    }

    #[test]
    fn find_last_skips_incomplete_trailing_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let mut file = File::create(&path).unwrap();
        writeln!(file, r#"{{"n":1}}"#).unwrap();
        writeln!(file, r#"{{"n":2}}"#).unwrap();
        write!(file, r#"{{"n":3"#).unwrap();
        drop(file);

        let found = find_last(&path, |line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|value| value.get("n").and_then(|n| n.as_i64()))
        });
        assert_eq!(found, Some(2));
        assert_eq!(find_last(&path, |_| None::<i64>), None);
    }

    #[test]
    fn find_last_grows_the_window() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log.jsonl");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "wanted").unwrap();
        let filler = "x".repeat(1000);
        for _ in 0..200 {
            writeln!(file, "{filler}").unwrap();
        }
        drop(file);
        let found = find_last(&path, |line| (line == "wanted").then_some(true));
        assert_eq!(found, Some(true));
    }

    #[test]
    fn newest_file_respects_suffix_and_depth() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        fs::create_dir_all(&nested).unwrap();
        fs::write(dir.path().join("old.jsonl"), "x").unwrap();
        fs::write(dir.path().join("note.txt"), "x").unwrap();
        fs::write(nested.join("deep.JSONL"), "x").unwrap();

        let old = SystemTime::now() - std::time::Duration::from_secs(3600);
        File::options()
            .write(true)
            .open(dir.path().join("old.jsonl"))
            .unwrap()
            .set_modified(old)
            .unwrap();

        let (path, _) = newest_file(dir.path(), &[".jsonl"], 4).unwrap();
        assert!(path.ends_with("deep.JSONL"));
        let (shallow, _) = newest_file(dir.path(), &[".jsonl"], 0).unwrap();
        assert!(shallow.ends_with("old.jsonl"));
        assert!(newest_file(dir.path(), &[".zst"], 4).is_none());
        assert!(newest_file(&dir.path().join("missing"), &[".jsonl"], 4).is_none());
    }
}
