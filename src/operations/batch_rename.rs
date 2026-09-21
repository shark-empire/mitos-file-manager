use crate::operations::jobs::{JobHandle, JobMessage};
use async_channel::Sender;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Compute a new filename from a pattern.
///
/// Supported tokens:
///   {name}   → original filename stem (without extension)
///   {ext}    → ".extension" (with dot), or empty if none
///   {n}      → sequential counter (unpadded)
///   {0}      → counter padded to width 1
///   {00}     → counter padded to width 2
///   {000}    → counter padded to width 3
///   {parent} → parent directory name
///   {date}   → current local date as YYYY-MM-DD
///   {time}   → current local time as HH-MM-SS
pub fn compute_new_name(
    pattern: &str,
    index: usize,
    start_number: u64,
    original_name: &str,
    parent_dir: &Path,
) -> String {
    let path = Path::new(original_name);

    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let ext = path
        .extension()
        .map(|s| format!(".{}", s.to_string_lossy()))
        .unwrap_or_default();

    let parent_name = parent_dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "folder".to_string());

    let number = start_number + index as u64;

    let (date_str, time_str) = current_datetime_strings();

    let mut result = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '{' {
            if let Some(close_offset) = chars[i..].iter().position(|&c| c == '}') {
                let token: String = chars[i + 1..i + close_offset].iter().collect();

                match token.as_str() {
                    "name" => result.push_str(&stem),
                    "ext" => result.push_str(&ext),
                    "parent" => result.push_str(&parent_name),
                    "date" => result.push_str(&date_str),
                    "time" => result.push_str(&time_str),
                    "n" => result.push_str(&number.to_string()),
                    _ => {
                        if !token.is_empty() && token.chars().all(|c| c == '0') {
                            let width = token.len();
                            result.push_str(&format!("{:0>width$}", number, width = width));
                        } else {
                            result.push('{');
                            result.push_str(&token);
                            result.push('}');
                        }
                    }
                }

                i += close_offset + 1;
            } else {
                result.push(chars[i]);
                i += 1;
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

fn current_datetime_strings() -> (String, String) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // What people expect `{date}` / `{time}` to mean is their wall clock,
    // not UTC. If the C library can't tell us the local time for some
    // reason, fall back to computing UTC by hand below.
    if let Some(local) = local_datetime_strings(secs) {
        return local;
    }

    let (year, month, day) = civil_from_days((secs / 86400) as i64);
    let hour = (secs % 86400) / 3600;
    let minute = (secs % 3600) / 60;
    let second = secs % 60;

    let date_str = format!("{:04}-{:02}-{:02}", year, month, day);
    let time_str = format!("{:02}-{:02}-{:02}", hour, minute, second);

    (date_str, time_str)
}

/// `("YYYY-MM-DD", "HH-MM-SS")` in the local time zone, via `localtime_r`.
fn local_datetime_strings(unix_secs: u64) -> Option<(String, String)> {
    let timestamp = unix_secs as libc::time_t;

    // SAFETY: `libc::tm` is plain old data for which all-zero bytes is a
    // valid value (the one pointer some platforms keep in it, `tm_zone`, is
    // a nullable raw pointer). `localtime_r` reads `timestamp` and writes
    // only into the `tm` handed to it -- both are live, exclusively owned
    // locals for the whole call -- and, unlike plain `localtime`, uses no
    // shared static buffer, so it is safe to call from any thread.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();

        if libc::localtime_r(&timestamp, &mut tm).is_null() {
            return None;
        }

        tm
    };

    Some((
        format!(
            "{:04}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday
        ),
        format!("{:02}-{:02}-{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec),
    ))
}

/// Howard Hinnant's civil_from_days algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

/// Validate a set of renames. Returns a list of warnings.
pub fn validate_renames(renames: &[(PathBuf, PathBuf)]) -> Vec<String> {
    let mut warnings = Vec::new();

    let mut targets: Vec<&Path> = Vec::new();

    for (source, target) in renames {
        if target.exists() && source != target && !is_same_file(source, target) {
            let is_source_being_renamed = renames.iter().any(|(s, _)| s == target);
            if !is_source_being_renamed {
                warnings.push(format!(
                    "Target already exists: {}",
                    target.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }

        if targets.contains(&target.as_path()) {
            warnings.push(format!(
                "Duplicate target: {}",
                target.file_name().unwrap_or_default().to_string_lossy()
            ));
        }

        targets.push(target.as_path());
    }

    warnings
}

/// `true` if both paths are the very same file on disk. That's how a
/// case-only rename ("a.txt" -> "A.txt") looks on a case-insensitive
/// filesystem such as FAT: the "existing" target is the source itself, which
/// is fine, not a collision.
fn is_same_file(a: &Path, b: &Path) -> bool {
    match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

pub fn start_batch_rename_job(
    renames: Vec<(PathBuf, PathBuf)>,
    sender: Sender<JobMessage>,
) -> JobHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));

    let handle = JobHandle {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };

    thread::spawn(move || {
        let result = run_batch_rename(&renames, &sender, cancel, pause);

        let _ = sender.send_blocking(JobMessage::Finished { result });
    });

    handle
}

/// One rename in flight: (original path, temporary path, final path).
type Staged = (PathBuf, PathBuf, PathBuf);

fn run_batch_rename(
    renames: &[(PathBuf, PathBuf)],
    sender: &Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
) -> Result<usize, String> {
    // The dialog already refuses to start with these problems, but the job
    // is the one doing the damage if they slip through, so it checks too.
    let problems = validate_renames(renames);

    if !problems.is_empty() {
        return Err(problems.join("; "));
    }

    let sources: Vec<PathBuf> = renames.iter().map(|(source, _)| source.clone()).collect();

    crate::filesystem::protection::ensure_modifiable(&sources).map_err(|err| err.to_string())?;

    let total = renames.len() as u64;

    let _ = sender.send_blocking(JobMessage::Started {
        label: "Renaming".to_string(),
        total,
        bytes: false,
    });

    let mut progress = RenameProgress {
        sender: sender.clone(),
        cancel,
        pause,
        total,
        processed: 0,
        last_sent: Instant::now(),
    };

    // Phase 1: move every source to a temporary name, so renames that swap
    // or chain names ("a" -> "b" while "b" -> "c") can't overwrite each
    // other half-way through.
    let mut staged: Vec<Staged> = Vec::with_capacity(renames.len());

    for (index, (source, target)) in renames.iter().enumerate() {
        if let Err(err) = progress.check() {
            roll_back(&staged, 0);
            return Err(err.to_string());
        }

        let dir = source.parent().unwrap_or_else(|| Path::new("."));
        let temp = temp_path_for(dir, index);

        if let Err(err) = fs::rename(source, &temp) {
            roll_back(&staged, 0);
            return Err(err.to_string());
        }

        staged.push((source.clone(), temp, target.clone()));
    }

    // Phase 2: move each temporary name to its final name. This is the part
    // the progress bar counts -- phase 1 is just staging.
    for (index, (_, temp, target)) in staged.iter().enumerate() {
        let step = progress.check().and_then(|()| fs::rename(temp, target));

        if let Err(err) = step {
            roll_back(&staged, index);
            return Err(err.to_string());
        }

        progress.add(1);
    }

    Ok(renames.len())
}

/// A name for the staging step that nothing else in `dir` is using -- a
/// hidden leftover from an earlier crashed run must not be overwritten.
fn temp_path_for(dir: &Path, index: usize) -> PathBuf {
    let pid = std::process::id();
    let mut attempt = 0u32;

    loop {
        let candidate = dir.join(format!(".mitos_rename_{pid}_{index}_{attempt}"));

        if fs::symlink_metadata(&candidate).is_err() {
            return candidate;
        }

        attempt += 1;
    }
}

/// Best-effort undo after a failure or cancel part-way through: entries
/// before `placed` already sit at their final name, the rest are still at
/// their temporary one. Either way they go back to their original name, so
/// nothing is ever left stranded as `.mitos_rename_...`.
fn roll_back(staged: &[Staged], placed: usize) {
    for (index, (original, temp, target)) in staged.iter().enumerate() {
        let current = if index < placed { target } else { temp };

        let _ = fs::rename(current, original);
    }
}

struct RenameProgress {
    sender: Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    total: u64,
    processed: u64,
    last_sent: Instant,
}

impl RenameProgress {
    /// Honour the progress dialog's Cancel / Pause buttons: blocks while
    /// paused, and errors once cancelled.
    fn check(&self) -> std::io::Result<()> {
        check_cancel_and_pause(&self.cancel, &self.pause)
    }

    fn add(&mut self, amount: u64) {
        self.processed += amount;

        if self.last_sent.elapsed().as_millis() >= 80 {
            let _ = self.sender.send_blocking(JobMessage::Progress {
                label: "Renaming".to_string(),
                processed: self.processed,
                total: self.total,
                bytes: false,
            });

            self.last_sent = Instant::now();
        }
    }
}

fn check_cancel_and_pause(cancel: &AtomicBool, pause: &AtomicBool) -> std::io::Result<()> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Interrupted,
                "Cancelled",
            ));
        }

        if !pause.load(Ordering::Relaxed) {
            return Ok(());
        }

        thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn civil_from_days_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11016), (2000, 2, 29));
        assert_eq!(civil_from_days(19000), (2022, 1, 8));
    }

    #[test]
    fn tokens_expand_as_documented() {
        let parent = Path::new("/home/u/Pics");

        assert_eq!(
            compute_new_name("Photo_{000}{ext}", 0, 1, "IMG_7.jpg", parent),
            "Photo_001.jpg"
        );
        assert_eq!(
            compute_new_name("{name}-{n}", 4, 10, "IMG_7.jpg", parent),
            "IMG_7-14"
        );
        assert_eq!(
            compute_new_name("{parent}_{00}", 2, 0, "x", parent),
            "Pics_02"
        );
        // Unknown tokens and unclosed braces are left exactly as typed.
        assert_eq!(compute_new_name("{nope}", 0, 0, "x", parent), "{nope}");
        assert_eq!(compute_new_name("a{b", 0, 0, "x", parent), "a{b");
    }

    #[test]
    fn date_and_time_tokens_have_the_documented_shape() {
        let (date, time) = current_datetime_strings();

        assert_eq!(date.len(), 10);
        assert_eq!(&date[4..5], "-");
        assert_eq!(&date[7..8], "-");
        assert_eq!(time.len(), 8);
        assert_eq!(&time[2..3], "-");
        assert_eq!(&time[5..6], "-");
    }

    #[test]
    fn validation_flags_clashes_and_duplicates_but_allows_swaps() {
        let dir = scratch_dir("rename-validate");
        let (a, b, c) = (dir.join("a"), dir.join("b"), dir.join("c"));
        fs::write(&a, "A").unwrap();
        fs::write(&b, "B").unwrap();

        // Swapping two names is fine: each target is another rename's source.
        assert!(validate_renames(&[(a.clone(), b.clone()), (b.clone(), a.clone())]).is_empty());

        // Renaming onto a file that isn't part of the batch is not.
        assert!(!validate_renames(&[(a.clone(), b.clone())]).is_empty());

        // Two sources onto one new name is not either.
        assert!(!validate_renames(&[(a.clone(), c.clone()), (b.clone(), c.clone())]).is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    fn run(renames: &[(PathBuf, PathBuf)], cancelled: bool) -> Result<usize, String> {
        let (sender, _receiver) = async_channel::unbounded();

        run_batch_rename(
            renames,
            &sender,
            Arc::new(AtomicBool::new(cancelled)),
            Arc::new(AtomicBool::new(false)),
        )
    }

    #[test]
    fn a_batch_that_swaps_two_names_ends_up_swapped() {
        let dir = scratch_dir("rename-swap");
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::write(&a, "was a").unwrap();
        fs::write(&b, "was b").unwrap();

        let done = run(&[(a.clone(), b.clone()), (b.clone(), a.clone())], false);

        assert_eq!(done, Ok(2));
        assert_eq!(fs::read_to_string(&a).unwrap(), "was b");
        assert_eq!(fs::read_to_string(&b).unwrap(), "was a");

        // No staging leftovers.
        let leftovers = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".mitos_rename"))
            .count();
        assert_eq!(leftovers, 0);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failure_part_way_puts_every_file_back() {
        let dir = scratch_dir("rename-rollback");
        let (a, b) = (dir.join("a"), dir.join("b"));
        fs::write(&a, "was a").unwrap();
        fs::write(&b, "was b").unwrap();

        // The second rename targets a folder that doesn't exist, so it fails
        // after the first has already been placed.
        let broken_target = dir.join("no-such-folder").join("b2");
        let done = run(&[(a.clone(), dir.join("a2")), (b.clone(), broken_target)], false);

        assert!(done.is_err());
        assert_eq!(fs::read_to_string(&a).unwrap(), "was a");
        assert_eq!(fs::read_to_string(&b).unwrap(), "was b");
        assert!(!dir.join("a2").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_cancelled_batch_changes_nothing() {
        let dir = scratch_dir("rename-cancel");
        let a = dir.join("a");
        fs::write(&a, "was a").unwrap();

        let done = run(&[(a.clone(), dir.join("a2"))], true);

        assert!(done.is_err());
        assert!(a.exists());
        assert!(!dir.join("a2").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
