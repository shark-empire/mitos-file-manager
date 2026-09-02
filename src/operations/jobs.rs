use crate::operations::{unique_destination, PendingOp};
use async_channel::Sender;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::symlink as symlink_unix;

#[derive(Clone, Copy, PartialEq)]
pub enum ConflictPolicy {
    KeepBoth,
    Replace,
    SkipExisting,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ConflictAction {
    Skip,
    Replace,
    KeepBoth,
}

#[derive(Clone)]
pub struct PasteTask {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub action: ConflictAction,
}

#[derive(Clone, Debug)]
pub enum JobMessage {
    Started {
        label: String,
        total: u64,
        bytes: bool,
    },
    Progress {
        label: String,
        processed: u64,
        total: u64,
        bytes: bool,
    },
    Finished {
        result: Result<usize, String>,
    },
}

pub struct JobHandle {
    pub cancel: Arc<AtomicBool>,
    pub pause: Arc<AtomicBool>,
}

struct ProgressState {
    sender: Sender<JobMessage>,
    cancel: Arc<AtomicBool>,
    pause: Arc<AtomicBool>,
    label: String,
    total: u64,
    processed: u64,
    bytes: bool,
    last_sent: Instant,
}

impl ProgressState {
    fn new(
        sender: Sender<JobMessage>,
        cancel: Arc<AtomicBool>,
        pause: Arc<AtomicBool>,
        label: String,
        total: u64,
        bytes: bool,
    ) -> Self {
        Self {
            sender,
            cancel,
            pause,
            label,
            total,
            processed: 0,
            bytes,
            last_sent: Instant::now(),
        }
    }

    fn add(&mut self, amount: u64) {
        self.processed += amount;
        self.maybe_send();
    }

    fn set(&mut self, value: u64) {
        self.processed = value;
        self.maybe_send();
    }

    fn maybe_send(&mut self) {
        if self.last_sent.elapsed().as_millis() >= 80 {
            let _ = self.sender.send_blocking(JobMessage::Progress {
                label: self.label.clone(),
                processed: self.processed,
                total: self.total,
                bytes: self.bytes,
            });

            self.last_sent = Instant::now();
        }
    }
}

pub fn start_paste_job(
    operation: PendingOp,
    tasks: Vec<PasteTask>,
    sender: Sender<JobMessage>,
) -> JobHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));

    let handle = JobHandle {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };

    let label = match operation {
        PendingOp::Copy => "Copying".to_string(),
        PendingOp::Move => "Moving".to_string(),
    };

    thread::spawn(move || {
        let result = (|| -> Result<usize, String> {
            let active_tasks: Vec<PasteTask> = tasks
                .into_iter()
                .filter(|task| task.action != ConflictAction::Skip && task.source.exists())
                .collect();

            let total =
                calculate_size_for_tasks(&active_tasks, &cancel).map_err(|err| err.to_string())?;

            let _ = sender.send_blocking(JobMessage::Started {
                label: label.clone(),
                total,
                bytes: true,
            });

            let mut state = ProgressState::new(
                sender.clone(),
                cancel.clone(),
                pause.clone(),
                label.clone(),
                total,
                true,
            );

            let mut completed = 0;

            for task in &active_tasks {
                check_cancel_and_pause(&state.cancel, &state.pause)
                    .map_err(|err| err.to_string())?;

                let source = &task.source;

                if !source.exists() {
                    continue;
                }

                let destination_dir = task.destination.parent().unwrap_or_else(|| Path::new("/"));

                if destination_dir.starts_with(source) || source.as_path() == destination_dir {
                    continue;
                }

                if source.parent() == Some(destination_dir) {
                    continue;
                }

                let mut target = task.destination.clone();

                if task.action == ConflictAction::KeepBoth {
                    target = unique_destination(&target);
                }

                if task.action == ConflictAction::Replace && target.exists() {
                    remove_all_with_progress(&target, &mut state).map_err(|err| err.to_string())?;
                }

                match operation {
                    PendingOp::Copy => copy_path_with_progress(source, &target, &mut state),
                    PendingOp::Move => move_path_with_progress(source, &target, &mut state),
                }
                .map_err(|err| err.to_string())?;

                completed += 1;
            }

            state.set(total);

            Ok(completed)
        })();

        let _ = sender.send_blocking(JobMessage::Finished { result });
    });

    handle
}

pub fn start_trash_job(paths: Vec<PathBuf>, sender: Sender<JobMessage>) -> JobHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let pause = Arc::new(AtomicBool::new(false));

    let handle = JobHandle {
        cancel: cancel.clone(),
        pause: pause.clone(),
    };

    thread::spawn(move || {
        let result = (|| -> Result<usize, String> {
            let total = paths.len() as u64;

            let _ = sender.send_blocking(JobMessage::Started {
                label: "Moving to trash".to_string(),
                total,
                bytes: false,
            });

            let mut state = ProgressState::new(
                sender.clone(),
                cancel.clone(),
                pause.clone(),
                "Moving to trash".to_string(),
                total,
                false,
            );

            let mut completed = 0;

            for (index, path) in paths.iter().enumerate() {
                check_cancel_and_pause(&state.cancel, &state.pause)
                    .map_err(|err| err.to_string())?;

                crate::operations::trash::delete(path).map_err(|err| err.to_string())?;

                completed += 1;
                state.set((index + 1) as u64);
            }

            Ok(completed)
        })();

        let _ = sender.send_blocking(JobMessage::Finished { result });
    });

    handle
}

fn check_cancel_and_pause(cancel: &AtomicBool, pause: &AtomicBool) -> io::Result<()> {
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
        }

        if !pause.load(Ordering::Relaxed) {
            return Ok(());
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn calculate_size_for_tasks(tasks: &[PasteTask], cancel: &AtomicBool) -> io::Result<u64> {
    let mut total = 0;

    for task in tasks {
        if cancel.load(Ordering::Relaxed) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
        }

        total += path_size(&task.source, cancel)?;
    }

    Ok(total)
}

fn path_size(path: &Path, cancel: &AtomicBool) -> io::Result<u64> {
    if cancel.load(Ordering::Relaxed) {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
    }

    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        let mut total = 0;

        for entry in fs::read_dir(path)? {
            if cancel.load(Ordering::Relaxed) {
                return Err(io::Error::new(io::ErrorKind::Interrupted, "Cancelled"));
            }

            let entry = entry?;
            total += path_size(&entry.path(), cancel)?;
        }

        Ok(total)
    } else {
        Ok(metadata.len())
    }
}

fn copy_path_with_progress(
    source: &Path,
    destination: &Path,
    state: &mut ProgressState,
) -> io::Result<()> {
    check_cancel_and_pause(&state.cancel, &state.pause)?;

    let metadata = fs::symlink_metadata(source)?;

    if metadata.is_dir() {
        copy_dir_with_progress(source, destination, state)
    } else if metadata.file_type().is_symlink() {
        copy_symlink(source, destination)
    } else {
        copy_file_with_progress(source, destination, state)
    }
}

fn copy_dir_with_progress(
    source: &Path,
    destination: &Path,
    state: &mut ProgressState,
) -> io::Result<()> {
    check_cancel_and_pause(&state.cancel, &state.pause)?;

    fs::create_dir_all(destination)?;

    for entry in fs::read_dir(source)? {
        check_cancel_and_pause(&state.cancel, &state.pause)?;

        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_with_progress(&entry.path(), &target, state)?;
        } else if file_type.is_symlink() {
            copy_symlink(&entry.path(), &target)?;
        } else {
            copy_file_with_progress(&entry.path(), &target, state)?;
        }
    }

    Ok(())
}

fn copy_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    let target = fs::read_link(source)?;

    #[cfg(unix)]
    symlink_unix(target, destination)?;

    #[cfg(not(unix))]
    {
        let _ = target;
        return Err(io::Error::new(
            io::ErrorKind::Other,
            "Symlink copying is not supported on this platform",
        ));
    }

    Ok(())
}

fn copy_file_with_progress(
    source: &Path,
    destination: &Path,
    state: &mut ProgressState,
) -> io::Result<()> {
    let result = copy_file_chunks(source, destination, state);

    if result.is_err() {
        let _ = fs::remove_file(destination);
    }

    result
}

fn copy_file_chunks(
    source: &Path,
    destination: &Path,
    state: &mut ProgressState,
) -> io::Result<()> {
    check_cancel_and_pause(&state.cancel, &state.pause)?;

    let mut reader = fs::File::open(source)?;
    let mut writer = fs::File::create(destination)?;

    let mut buffer = [0u8; 64 * 1024];

    loop {
        check_cancel_and_pause(&state.cancel, &state.pause)?;

        let read = reader.read(&mut buffer)?;

        if read == 0 {
            break;
        }

        writer.write_all(&buffer[..read])?;
        state.add(read as u64);
    }

    writer.flush()?;

    if let Ok(metadata) = fs::metadata(source) {
        let _ = fs::set_permissions(destination, metadata.permissions());
    }

    Ok(())
}

fn move_path_with_progress(
    source: &Path,
    destination: &Path,
    state: &mut ProgressState,
) -> io::Result<()> {
    check_cancel_and_pause(&state.cancel, &state.pause)?;

    match fs::rename(source, destination) {
        Ok(_) => Ok(()),
        Err(err) => {
            const EXDEV: i32 = 18;

            if err.raw_os_error() == Some(EXDEV) {
                copy_path_with_progress(source, destination, state)?;
                remove_all_with_progress(source, state).map_err(|err| {
                    std::io::Error::new(std::io::ErrorKind::Other, err.to_string())
                })?;

                Ok(())
            } else {
                Err(err)
            }
        }
    }
}

fn remove_all_with_progress(path: &Path, state: &mut ProgressState) -> io::Result<()> {
    check_cancel_and_pause(&state.cancel, &state.pause)?;

    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        for entry in fs::read_dir(path)? {
            check_cancel_and_pause(&state.cancel, &state.pause)?;

            let entry = entry?;
            remove_all_with_progress(&entry.path(), state)?;
        }

        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}
