//! Administrator ("root") operations.
//!
//! The file manager itself never runs as root. When something fails with
//! "Permission denied" the user is asked whether to retry it as
//! administrator; if they agree, this same executable is run again through
//! an elevation command (`pkexec` by default) in a tiny, window-less mode:
//!
//! ```text
//! pkexec /path/to/mitos-file-manager --privileged <operation> <arguments...>
//! ```
//!
//! That mode (`run_helper`) performs exactly one operation from the fixed
//! list below and exits. There is no shell and no arbitrary command: every
//! argument is a path that must be absolute and free of `..`, and anything
//! that deletes, moves or renames goes through the same protected-path
//! check the GUI uses.
//!
//! The elevation command can be swapped without recompiling by setting
//! `MITOS_ELEVATE_COMMAND` (e.g. to the MITOS permission service's own
//! prompt); the helper is then run as `<command...> <this exe> --privileged ...`.

use crate::filesystem::protection;
use crate::operations::{copy, move_op, occupied, unique_destination};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

/// The first argument that switches the executable into helper mode.
pub const FLAG: &str = "--privileged";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PasteKind {
    Copy,
    Move,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PasteEntry {
    /// Overwrite whatever is at `target` (otherwise pick a free "name (1)").
    pub replace: bool,
    pub source: PathBuf,
    pub target: PathBuf,
}

/// Everything the elevated helper can do.
#[derive(Clone, Debug, PartialEq)]
pub enum Operation {
    Delete(Vec<PathBuf>),
    Rename { from: PathBuf, to: PathBuf },
    CreateFolder(PathBuf),
    CreateFile(PathBuf),
    Paste { kind: PasteKind, entries: Vec<PasteEntry> },
    Chmod { mode: u32, path: PathBuf },
}

impl Operation {
    /// The command-line arguments that ask the helper to do this.
    pub fn to_args(&self) -> Vec<String> {
        let text = |path: &Path| path.to_string_lossy().to_string();

        match self {
            Operation::Delete(paths) => std::iter::once("delete".to_string())
                .chain(paths.iter().map(|path| text(path)))
                .collect(),
            Operation::Rename { from, to } => {
                vec!["rename".to_string(), text(from), text(to)]
            }
            Operation::CreateFolder(path) => vec!["mkdir".to_string(), text(path)],
            Operation::CreateFile(path) => vec!["touch".to_string(), text(path)],
            Operation::Chmod { mode, path } => {
                vec!["chmod".to_string(), format!("{mode:o}"), text(path)]
            }
            Operation::Paste { kind, entries } => {
                let mut args = vec![
                    "paste".to_string(),
                    match kind {
                        PasteKind::Copy => "copy",
                        PasteKind::Move => "move",
                    }
                    .to_string(),
                ];

                for entry in entries {
                    args.push(if entry.replace { "replace" } else { "keep" }.to_string());
                    args.push(text(&entry.source));
                    args.push(text(&entry.target));
                }

                args
            }
        }
    }

    /// Parse and validate the arguments after `--privileged`.
    pub fn from_args(args: &[String]) -> Result<Operation, String> {
        let (verb, rest) = args.split_first().ok_or("No operation given")?;

        match verb.as_str() {
            "delete" => {
                if rest.is_empty() {
                    return Err("delete needs at least one path".to_string());
                }

                let paths = rest
                    .iter()
                    .map(|arg| absolute(arg))
                    .collect::<Result<Vec<_>, _>>()?;

                Ok(Operation::Delete(paths))
            }
            "rename" => match rest {
                [from, to] => Ok(Operation::Rename {
                    from: absolute(from)?,
                    to: absolute(to)?,
                }),
                _ => Err("rename needs a source and a destination".to_string()),
            },
            "mkdir" => match rest {
                [path] => Ok(Operation::CreateFolder(absolute(path)?)),
                _ => Err("mkdir needs exactly one path".to_string()),
            },
            "touch" => match rest {
                [path] => Ok(Operation::CreateFile(absolute(path)?)),
                _ => Err("touch needs exactly one path".to_string()),
            },
            "chmod" => match rest {
                [mode, path] => {
                    let mode = u32::from_str_radix(mode, 8)
                        .map_err(|_| format!("\"{mode}\" is not an octal permission mode"))?;

                    if mode > 0o7777 {
                        return Err(format!("{mode:o} is not a valid permission mode"));
                    }

                    Ok(Operation::Chmod {
                        mode,
                        path: absolute(path)?,
                    })
                }
                _ => Err("chmod needs a mode and a path".to_string()),
            },
            "paste" => {
                let (kind, entries) = rest.split_first().ok_or("paste needs copy or move")?;

                let kind = match kind.as_str() {
                    "copy" => PasteKind::Copy,
                    "move" => PasteKind::Move,
                    other => return Err(format!("paste: unknown kind \"{other}\"")),
                };

                if entries.is_empty() || entries.len() % 3 != 0 {
                    return Err("paste needs (keep|replace, source, target) triples".to_string());
                }

                let mut parsed = Vec::new();

                for chunk in entries.chunks(3) {
                    let replace = match chunk[0].as_str() {
                        "keep" => false,
                        "replace" => true,
                        other => return Err(format!("paste: unknown conflict mode \"{other}\"")),
                    };

                    parsed.push(PasteEntry {
                        replace,
                        source: absolute(&chunk[1])?,
                        target: absolute(&chunk[2])?,
                    });
                }

                Ok(Operation::Paste {
                    kind,
                    entries: parsed,
                })
            }
            other => Err(format!("Unknown operation \"{other}\"")),
        }
    }
}

/// A path argument must be absolute and must not climb with `..`.
fn absolute(arg: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(arg);

    if !path.is_absolute() {
        return Err(format!("\"{arg}\" is not an absolute path"));
    }

    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("\"{arg}\" contains \"..\""));
    }

    Ok(path)
}

/// Entry point for `mitos-file-manager --privileged ...` (the arguments
/// *after* the flag). Returns the process exit code.
pub fn run_helper(args: &[String]) -> i32 {
    match Operation::from_args(args).and_then(|operation| execute(&operation)) {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("{message}");
            1
        }
    }
}

/// Perform `operation` with whatever privileges this process has.
pub fn execute(operation: &Operation) -> Result<(), String> {
    match operation {
        Operation::Delete(paths) => {
            protection::ensure_modifiable(paths).map_err(|err| err.to_string())?;

            for path in paths {
                remove_any(path).map_err(|err| format!("{}: {err}", path.display()))?;
            }

            Ok(())
        }
        Operation::Rename { from, to } => {
            protection::ensure_modifiable(&[from.clone()]).map_err(|err| err.to_string())?;

            if occupied(to) {
                return Err(format!("\"{}\" already exists", to.display()));
            }

            fs::rename(from, to).map_err(|err| err.to_string())
        }
        Operation::CreateFolder(path) => fs::create_dir(path).map_err(|err| err.to_string()),
        Operation::CreateFile(path) => fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map(|_| ())
            .map_err(|err| err.to_string()),
        Operation::Chmod { mode, path } => {
            fs::set_permissions(path, fs::Permissions::from_mode(*mode))
                .map_err(|err| err.to_string())
        }
        Operation::Paste { kind, entries } => {
            if matches!(kind, PasteKind::Move) {
                let sources: Vec<PathBuf> = entries.iter().map(|e| e.source.clone()).collect();

                protection::ensure_modifiable(&sources).map_err(|err| err.to_string())?;
            }

            for entry in entries {
                let target = if entry.replace {
                    entry.target.clone()
                } else {
                    unique_destination(&entry.target)
                };

                // Pasting something onto itself must never delete it.
                if target == entry.source {
                    continue;
                }

                if target
                    .parent()
                    .map_or(false, |parent| parent.starts_with(&entry.source))
                {
                    return Err(format!(
                        "\"{}\" can't be placed inside itself",
                        entry.source.display()
                    ));
                }

                if entry.replace && occupied(&target) {
                    protection::ensure_modifiable(&[target.clone()])
                        .map_err(|err| err.to_string())?;
                    remove_any(&target).map_err(|err| err.to_string())?;
                }

                match kind {
                    PasteKind::Copy => copy::copy_path(&entry.source, &target),
                    PasteKind::Move => move_op::move_path(&entry.source, &target),
                }
                .map_err(|err| format!("{}: {err}", entry.source.display()))?;
            }

            Ok(())
        }
    }
}

/// Delete a file, link or whole folder tree. Links are removed as links --
/// `remove_dir_all` never follows a symlink out of the tree.
fn remove_any(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;

    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

/// The command (program and leading arguments) that elevates the helper:
/// `MITOS_ELEVATE_COMMAND` if set, otherwise `pkexec`.
pub fn elevation_command() -> Vec<String> {
    parse_elevation_command(std::env::var("MITOS_ELEVATE_COMMAND").ok().as_deref())
}

fn parse_elevation_command(value: Option<&str>) -> Vec<String> {
    let parts: Vec<String> = value
        .map(|text| text.split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();

    if parts.is_empty() {
        vec!["pkexec".to_string()]
    } else {
        parts
    }
}

/// Run `operation` as administrator: this blocks (the authentication prompt
/// belongs to the elevation command), so call it from a worker thread.
pub fn run_elevated(operation: &Operation) -> Result<(), String> {
    let exe = std::env::current_exe()
        .map_err(|err| format!("Couldn't find this program to re-run it: {err}"))?;

    let mut command_line = elevation_command();
    let program = command_line.remove(0);

    let output = Command::new(&program)
        .args(&command_line)
        .arg(&exe)
        .arg(FLAG)
        .args(operation.to_args())
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("Couldn't run {program}: {err}"))?;

    if output.status.success() {
        return Ok(());
    }

    let message = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !message.is_empty() {
        return Err(message);
    }

    // pkexec's own "it didn't happen" exit codes.
    Err(match output.status.code() {
        Some(126) => "Administrator authentication was cancelled or refused".to_string(),
        Some(127) => {
            "Couldn't ask for administrator permission (is a polkit authentication agent running?)"
                .to_string()
        }
        _ => format!("The administrator helper failed ({})", output.status),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn every_operation_survives_a_trip_through_the_command_line() {
        let operations = vec![
            Operation::Delete(vec![PathBuf::from("/a b/c"), PathBuf::from("/d")]),
            Operation::Rename {
                from: PathBuf::from("/x/old name"),
                to: PathBuf::from("/x/new"),
            },
            Operation::CreateFolder(PathBuf::from("/x/folder")),
            Operation::CreateFile(PathBuf::from("/x/file")),
            Operation::Chmod {
                mode: 0o755,
                path: PathBuf::from("/x/script"),
            },
            Operation::Paste {
                kind: PasteKind::Move,
                entries: vec![
                    PasteEntry {
                        replace: false,
                        source: PathBuf::from("/s/one"),
                        target: PathBuf::from("/t/one"),
                    },
                    PasteEntry {
                        replace: true,
                        source: PathBuf::from("/s/two"),
                        target: PathBuf::from("/t/two"),
                    },
                ],
            },
        ];

        for operation in operations {
            assert_eq!(Operation::from_args(&operation.to_args()), Ok(operation));
        }
    }

    #[test]
    fn arguments_that_could_do_something_unintended_are_rejected() {
        for bad in [
            args(&[]),
            args(&["frobnicate", "/x"]),
            args(&["delete"]),
            args(&["delete", "relative/path"]),
            args(&["delete", "/x/../etc"]),
            args(&["rename", "/only-one"]),
            args(&["chmod", "999", "/x"]),
            args(&["chmod", "17777", "/x"]),
            args(&["chmod", "644", "x"]),
            args(&["paste", "copy", "keep", "/a"]),
            args(&["paste", "sideways", "keep", "/a", "/b"]),
            args(&["paste", "copy", "maybe", "/a", "/b"]),
        ] {
            assert!(Operation::from_args(&bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn the_elevation_command_can_be_overridden_but_defaults_to_pkexec() {
        assert_eq!(parse_elevation_command(None), vec!["pkexec"]);
        assert_eq!(parse_elevation_command(Some("   ")), vec!["pkexec"]);
        assert_eq!(
            parse_elevation_command(Some("mitosvc-ctl elevate --")),
            vec!["mitosvc-ctl", "elevate", "--"]
        );
    }

    #[test]
    fn create_rename_and_delete_work_and_never_overwrite() {
        let dir = scratch_dir("priv-basic");

        execute(&Operation::CreateFolder(dir.join("folder"))).unwrap();
        assert!(dir.join("folder").is_dir());
        assert!(execute(&Operation::CreateFolder(dir.join("folder"))).is_err());

        execute(&Operation::CreateFile(dir.join("file"))).unwrap();
        fs::write(dir.join("file"), "keep").unwrap();
        assert!(execute(&Operation::CreateFile(dir.join("file"))).is_err());
        assert_eq!(fs::read_to_string(dir.join("file")).unwrap(), "keep");

        assert!(execute(&Operation::Rename {
            from: dir.join("folder"),
            to: dir.join("file"),
        })
        .is_err());

        execute(&Operation::Rename {
            from: dir.join("folder"),
            to: dir.join("renamed"),
        })
        .unwrap();
        assert!(dir.join("renamed").is_dir());

        execute(&Operation::Delete(vec![dir.join("renamed"), dir.join("file")])).unwrap();
        assert!(!dir.join("renamed").exists() && !dir.join("file").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn delete_does_not_follow_symlinks_and_refuses_protected_paths() {
        let root = scratch_dir("priv-delete");
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), "x").unwrap();

        let tree = root.join("tree");
        fs::create_dir(&tree).unwrap();
        std::os::unix::fs::symlink(&outside, tree.join("link")).unwrap();

        execute(&Operation::Delete(vec![tree.clone()])).unwrap();
        assert!(!tree.exists());
        assert!(outside.join("keep").exists());

        assert!(execute(&Operation::Delete(vec![PathBuf::from("/usr")])).is_err());
        assert!(Path::new("/usr").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn move_keeps_both_when_asked_and_replaces_when_asked() {
        let root = scratch_dir("priv-paste");
        let (src, dst) = (root.join("src"), root.join("dst"));
        fs::create_dir_all(&src).unwrap();
        fs::create_dir_all(&dst).unwrap();

        fs::write(src.join("a"), "new a").unwrap();
        fs::write(src.join("b"), "new b").unwrap();
        fs::write(dst.join("a"), "old a").unwrap();
        fs::write(dst.join("b"), "old b").unwrap();

        execute(&Operation::Paste {
            kind: PasteKind::Move,
            entries: vec![
                PasteEntry {
                    replace: false,
                    source: src.join("a"),
                    target: dst.join("a"),
                },
                PasteEntry {
                    replace: true,
                    source: src.join("b"),
                    target: dst.join("b"),
                },
            ],
        })
        .unwrap();

        assert_eq!(fs::read_to_string(dst.join("a")).unwrap(), "old a");
        assert_eq!(fs::read_to_string(dst.join("a (1)")).unwrap(), "new a");
        assert_eq!(fs::read_to_string(dst.join("b")).unwrap(), "new b");
        assert!(!src.join("a").exists() && !src.join("b").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn chmod_sets_exactly_the_requested_mode() {
        let dir = scratch_dir("priv-chmod");
        let file = dir.join("script");
        fs::write(&file, "#!/bin/sh").unwrap();

        execute(&Operation::Chmod {
            mode: 0o600,
            path: file.clone(),
        })
        .unwrap();

        assert_eq!(fs::metadata(&file).unwrap().permissions().mode() & 0o7777, 0o600);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_cannot_be_pasted_into_itself() {
        let root = scratch_dir("priv-self");
        let folder = root.join("f");
        fs::create_dir_all(folder.join("inner")).unwrap();

        let result = execute(&Operation::Paste {
            kind: PasteKind::Copy,
            entries: vec![PasteEntry {
                replace: false,
                source: folder.clone(),
                target: folder.join("inner").join("f"),
            }],
        });

        assert!(result.is_err());

        let _ = fs::remove_dir_all(&root);
    }
}
