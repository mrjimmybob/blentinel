//! The `file` check (M5-03, D-79): the newest file at a path, how old it is
//! and how big. "Did last night's backup write a file?"
//!
//! The probe only reads names, sizes and modification times, with `std::fs`
//! (FP-2): never a file's content. How old is too old is the hub's threshold
//! (proposal 0006 §1.5); a file smaller than the check's `min_bytes` is a
//! failed check, like a folder without any matching file.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use common::checks::{Metric, metric};

use super::Outcome;

/// The most entries one check looks at: a scan can't be stopped from
/// outside once it runs, so it stops itself.
const MAX_ENTRIES: usize = 200_000;
/// How deep a check of a folder and its subfolders goes.
const MAX_DEPTH: usize = 16;

const SECONDS_PER_HOUR: f64 = 3600.0;
const BYTES_PER_MB: f64 = 1024.0 * 1024.0;

/// The newest file a scan found.
#[derive(Debug, Clone, PartialEq)]
struct Newest {
    name: String,
    modified: SystemTime,
    bytes: u64,
}

/// What a scan found.
#[derive(Debug, Clone, PartialEq, Default)]
struct Found {
    /// The files that match.
    files: u64,
    newest: Option<Newest>,
    /// It stopped at [`MAX_ENTRIES`].
    cut_short: bool,
}

impl Found {
    fn add(&mut self, name: String, metadata: &fs::Metadata) {
        self.files += 1;
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        if self.newest.as_ref().is_none_or(|n| modified > n.modified) {
            self.newest = Some(Newest {
                name,
                modified,
                bytes: metadata.len(),
            });
        }
    }
}

/// Look at `path` (a file, or a folder's files that match `pattern`) and
/// judge the newest.
pub async fn check(path: &str, pattern: &str, recursive: bool, min_bytes: Option<u64>) -> Outcome {
    let (root, wanted) = (PathBuf::from(path), pattern.to_string());
    let scanned = tokio::task::spawn_blocking(move || scan(&root, &wanted, recursive)).await;
    match scanned {
        Ok(Ok(found)) => judge(&found, pattern, min_bytes, SystemTime::now()),
        Ok(Err(e)) => Outcome::down(format!("Can't read {path}: {e}")),
        Err(_) => Outcome::down(format!("Reading {path} failed")),
    }
}

/// The files at `path`: itself if it is a file, else the files of the folder
/// (and, with `recursive`, of its subfolders) whose names match `pattern`.
/// A subfolder or entry that can't be read is left out.
fn scan(path: &Path, pattern: &str, recursive: bool) -> io::Result<Found> {
    let mut found = Found::default();
    let metadata = fs::metadata(path)?;
    if metadata.is_file() {
        let name = path.file_name().unwrap_or(path.as_os_str());
        found.add(name.to_string_lossy().into_owned(), &metadata);
        return Ok(found);
    }
    let mut budget = MAX_ENTRIES;
    let mut folders = vec![(path.to_path_buf(), 0usize)];
    while let Some((folder, depth)) = folders.pop() {
        let entries = match fs::read_dir(&folder) {
            Ok(entries) => entries,
            // The folder asked for must be readable; one below it needn't be.
            Err(e) if depth == 0 => return Err(e),
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            if budget == 0 {
                found.cut_short = true;
                return Ok(found);
            }
            budget -= 1;
            // A link's own type: linked folders aren't followed (no loops).
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if recursive && depth < MAX_DEPTH {
                    folders.push((entry.path(), depth + 1));
                }
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if !matches(pattern, &name) {
                continue;
            }
            // Through a link, to the file it points at.
            if let Ok(metadata) = fs::metadata(entry.path())
                && metadata.is_file()
            {
                found.add(name, &metadata);
            }
        }
    }
    Ok(found)
}

/// Does `name` match `pattern`? `*` is any run of characters, `?` any one;
/// an empty pattern matches everything. Letter case doesn't matter (it
/// doesn't on Windows, and `*.BAK` should find `.bak` anywhere).
pub(super) fn matches(pattern: &str, name: &str) -> bool {
    if pattern.is_empty() {
        return true;
    }
    let lower = |text: &str| {
        text.chars()
            .flat_map(char::to_lowercase)
            .collect::<Vec<_>>()
    };
    let (pattern, name) = (lower(pattern), lower(name));
    // Where to go back to after a `*` that took too little.
    let (mut p, mut n, mut star, mut resume) = (0, 0, None, 0);
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            resume = n;
            p += 1;
        } else if let Some(at) = star {
            p = at + 1;
            resume += 1;
            n = resume;
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// `512 bytes`, `340 KB`, `1.4 GB`.
fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value >= 100.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// `40 minutes`, `5.2 hours`, `3.1 days`.
fn age(hours: f64) -> String {
    if hours < 1.0 {
        format!("{:.0} minutes", hours * 60.0)
    } else if hours < 48.0 {
        format!("{hours:.1} hours")
    } else {
        format!("{:.1} days", hours / 24.0)
    }
}

/// What a scan is worth at `now`: Down without a file, or with a newest file
/// smaller than `min_bytes`; else Up. With the newest file's age and size.
fn judge(found: &Found, pattern: &str, min_bytes: Option<u64>, now: SystemTime) -> Outcome {
    let Some(newest) = &found.newest else {
        return Outcome::down(match pattern {
            "" => "No file there".to_string(),
            pattern => format!("No file matches {pattern}"),
        });
    };
    // A file dated in the future (a clock that was wrong) is new.
    let hours = now
        .duration_since(newest.modified)
        .map_or(0.0, |d| d.as_secs_f64() / SECONDS_PER_HOUR);
    let metrics = vec![
        Metric::new(metric::AGE, hours, metric::HOURS),
        Metric::new(
            metric::SIZE,
            newest.bytes as f64 / BYTES_PER_MB,
            metric::MEGABYTES,
        ),
        Metric::new(metric::FILES, found.files as f64, ""),
    ];
    let among = match found.files {
        1 => String::new(),
        n if found.cut_short => format!(" (the newest of the first {n} files)"),
        n => format!(" (the newest of {n} files)"),
    };
    let what = format!(
        "{}: {} old, {}{among}",
        newest.name,
        age(hours),
        size(newest.bytes)
    );
    match min_bytes {
        Some(min) if newest.bytes < min => {
            Outcome::down(format!("{what}; smaller than {}", size(min))).with_metrics(metrics)
        }
        _ => Outcome::up(what).with_metrics(metrics),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::checks::Health;
    use std::time::Duration;

    fn write(folder: &Path, name: &str, bytes: usize, hours_old: u64) {
        let path = folder.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, vec![0u8; bytes]).unwrap();
        let modified = SystemTime::now() - Duration::from_secs(hours_old * 3600);
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }

    fn metric_of(outcome: &Outcome, key: &str) -> f64 {
        outcome.metrics.iter().find(|m| m.key == key).unwrap().value
    }

    #[test]
    fn patterns_match_names_whatever_their_case() {
        for (pattern, name, expected) in [
            ("", "anything", true),
            ("*.bak", "nightly.bak", true),
            ("*.BAK", "nightly.bak", true),
            ("*.bak", "nightly.bak.tmp", false),
            ("backup-*.zip", "backup-2026-10-11.zip", true),
            ("backup-*.zip", "backup-.zip", true),
            ("backup-*.zip", "Backup-1.zip.part", false),
            ("db_??.dump", "db_07.dump", true),
            ("db_??.dump", "db_7.dump", false),
            ("*a*b*", "xxaxxbxx", true),
            ("*a*b*", "xxbxxaxx", false),
            ("exact.txt", "exact.txt", true),
            ("exact.txt", "exact.txt2", false),
            ("*", "", true),
            ("?", "", false),
        ] {
            assert_eq!(matches(pattern, name), expected, "{pattern} / {name}");
        }
    }

    #[tokio::test]
    async fn the_newest_matching_file_is_found_with_its_age_and_size() {
        let folder = tempfile::tempdir().unwrap();
        write(folder.path(), "monday.bak", 1024, 50);
        write(folder.path(), "tuesday.bak", 3 * 1024 * 1024, 5);
        write(folder.path(), "notes.txt", 10, 0);
        write(folder.path(), "old/sunday.bak", 2048, 1);
        let path = folder.path().to_str().unwrap();

        let outcome = check(path, "*.bak", false, None).await;
        assert_eq!(outcome.status, Health::Up, "{}", outcome.message);
        assert_eq!(
            outcome.message,
            "tuesday.bak: 5.0 hours old, 3.0 MB (the newest of 2 files)"
        );
        let keys: Vec<_> = outcome.metrics.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(
            keys,
            [metric::AGE, metric::SIZE, metric::FILES],
            "age first"
        );
        assert!((metric_of(&outcome, metric::AGE) - 5.0).abs() < 0.01);
        assert_eq!(metric_of(&outcome, metric::SIZE), 3.0);
        assert_eq!(metric_of(&outcome, metric::FILES), 2.0);

        // Subfolders only when asked.
        let deep = check(path, "*.bak", true, None).await;
        assert!(
            deep.message.starts_with("sunday.bak: 1.0 hours old"),
            "{}",
            deep.message
        );
        assert_eq!(metric_of(&deep, metric::FILES), 3.0);

        // Without a pattern: every file.
        let all = check(path, "", false, None).await;
        assert!(
            all.message
                .starts_with("notes.txt: 0 minutes old, 10 bytes"),
            "{}",
            all.message
        );
    }

    #[tokio::test]
    async fn a_file_that_is_too_small_or_missing_is_down() {
        let folder = tempfile::tempdir().unwrap();
        write(folder.path(), "nightly.bak", 2048, 3);
        let path = folder.path().to_str().unwrap();

        let small = check(path, "*.bak", false, Some(1024 * 1024)).await;
        assert_eq!(small.status, Health::Down);
        assert_eq!(
            small.message,
            "nightly.bak: 3.0 hours old, 2.0 KB; smaller than 1.0 MB"
        );
        assert_eq!(small.metrics.len(), 3, "measured anyway");
        let big_enough = check(path, "*.bak", false, Some(2048)).await;
        assert_eq!(big_enough.status, Health::Up);

        let none = check(path, "*.zip", false, None).await;
        assert_eq!(
            (none.status, none.message.as_str()),
            (Health::Down, "No file matches *.zip")
        );
        assert!(none.metrics.is_empty());

        let missing = check(&format!("{path}/nowhere"), "", false, None).await;
        assert_eq!(missing.status, Health::Down);
        assert!(
            missing.message.starts_with("Can't read "),
            "{}",
            missing.message
        );
    }

    #[tokio::test]
    async fn a_path_can_be_one_file() {
        let folder = tempfile::tempdir().unwrap();
        write(folder.path(), "database.dump", 4096, 30);
        let file = folder.path().join("database.dump");

        // The pattern is for folders: one file is that file.
        let outcome = check(file.to_str().unwrap(), "*.bak", false, None).await;
        assert_eq!(outcome.status, Health::Up);
        assert_eq!(outcome.message, "database.dump: 30.0 hours old, 4.0 KB");
    }

    #[test]
    fn ages_and_sizes_read_well() {
        assert_eq!(age(0.5), "30 minutes");
        assert_eq!(age(26.04), "26.0 hours");
        assert_eq!(age(72.0), "3.0 days");
        assert_eq!(size(0), "0 bytes");
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(size(250 * 1024 * 1024), "250 MB");
        assert_eq!(size(1_500_000_000), "1.4 GB");

        // A file from the future is new, not an error.
        let newest = Newest {
            name: "x".into(),
            modified: SystemTime::now() + Duration::from_secs(3600),
            bytes: 1,
        };
        let found = Found {
            files: 1,
            newest: Some(newest),
            cut_short: false,
        };
        assert_eq!(
            judge(&found, "", None, SystemTime::now()).metrics[0].value,
            0.0
        );
    }
}
