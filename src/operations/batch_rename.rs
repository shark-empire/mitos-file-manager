use crate::operations::jobs::{JobHandle, JobMessage};
use gtk::glib;
use std::fs;
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
///   {date}   → current date as YYYY-MM-DD
///   {time}   → current time as HH-MM-SS
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

    let (year, month, day) = civil_from_days((secs / 86400) as i64);
    let hour = (secs % 86400) / 3600;
    let minute = (secs % 3600) / 60;
    let second = secs % 60;

    let date_str = format!("{:04}-{:02}-{:02}", year, month, day);
    let time_str = format!("{:02}-{:02}-{:02}", hour, minute, second);

    (date_str, time_str)
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
        if target.exists() && source != target {
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

pub fn start_batch_rename_job(
    renames: Vec<(PathBuf, PathBuf)>,
    sender: glib::Sender<JobMessage>,
) -> JobHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));

    let handle = JobHandle {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };

    thread::spawn(move || {
        let result = (|| -> Result<usize, String> {
            let total = renames.len() as u64;

            let _ = sender.send(JobMessage::Started {
                label: "Renaming".to_string(),
                total,
                bytes: false,
            });

            let mut progress = RenameProgress {
                sender: sender.clone(),
                cancel: cancel.clone(),
                pause: pause.clone(),
                total,
                processed: 0,
                last_sent: Instant::now(),
            };

            // Phase 1: rename all sources to temporary names.
            let mut temp_paths: Vec<(PathBuf, PathBuf)> = Vec::new();

            for (index, (source, target)) in renames.iter().enumerate() {
                check_cancel_and_pause(&cancel, &pause).map_err(|e| e.to_string())?;

                let dir = source.parent().unwrap_or_else(|| Path::new("."));
                let temp_name = format!(".mitos_rename_tmp_{}", index);
                let temp_path = dir.join(&temp_name);

                fs::rename(source, &temp_path).map_err(|e| e.to_string())?;

                temp_paths.push((temp_path, target.clone()));
                progress.add(1);
            }

            // Phase 2: rename all temporary names to final targets.
            for (temp_path, target) in &temp_paths {
                check_cancel_and_pause(&cancel, &pause).map_err(|e| e.to_string())?;

                fs::rename(temp_path, target).map_err(|e| e.to_string())?;

                progress.add(1);
            }

            Ok(renames.len())
        })();

        let _ = sender.send(JobMessage::Finished { result });
    });

    handle
}

struct RenameProgress {
    sender: glib::Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    total: u64,
    processed: u64,
    last_sent: Instant,
}

impl RenameProgress {
    fn add(&mut self, amount: u64) {
        self.processed += amount;

        if self.last_sent.elapsed().as_millis() >= 80 {
            let _ = self.sender.send(JobMessage::Progress {
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

