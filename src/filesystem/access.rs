//! What the *current user* may actually do with a path -- as opposed to the
//! `rwxr-xr-x` string, which says nothing about who is asking. Used to grey
//! out New Folder / Paste in a folder that can't be written to, and to show
//! "you can read / write" in Properties.

use std::ffi::CString;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Access {
    pub read: bool,
    pub write: bool,
    pub execute: bool,
}

impl Access {
    /// "read, write" / "read only" / "no access" -- for display.
    pub fn describe(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();

        if self.read {
            parts.push("read");
        }
        if self.write {
            parts.push("write");
        }
        if self.execute {
            parts.push("execute");
        }

        match parts.as_slice() {
            [] => "no access".to_string(),
            ["read"] => "read only".to_string(),
            _ => parts.join(", "),
        }
    }
}

pub fn effective_access(path: &Path) -> Access {
    Access {
        read: check(path, libc::R_OK),
        write: check(path, libc::W_OK),
        execute: check(path, libc::X_OK),
    }
}

pub fn can_write(path: &Path) -> bool {
    check(path, libc::W_OK)
}

fn check(path: &Path, mode: libc::c_int) -> bool {
    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };

    // SAFETY: `c_path` is a valid NUL-terminated C string that outlives the
    // call, and `access(2)` only reads it and the file's permissions.
    unsafe { libc::access(c_path.as_ptr(), mode) == 0 }
}

/// User name for a numeric uid, from `/etc/passwd` -- or the number itself
/// if there's no entry (a container, a deleted account).
pub fn user_name(uid: u32) -> String {
    lookup_file("/etc/passwd", uid).unwrap_or_else(|| uid.to_string())
}

/// Group name for a numeric gid, from `/etc/group`.
pub fn group_name(gid: u32) -> String {
    lookup_file("/etc/group", gid).unwrap_or_else(|| gid.to_string())
}

fn lookup_file(file: &str, id: u32) -> Option<String> {
    let text = fs::read_to_string(file).ok()?;
    lookup_in(&text, id)
}

/// Find `id` in the third field of a `name:x:id:...` style file.
fn lookup_in(text: &str, id: u32) -> Option<String> {
    for line in text.lines() {
        let mut fields = line.split(':');

        let (Some(name), Some(_), Some(id_field)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };

        if id_field.parse::<u32>().ok() == Some(id) {
            return Some(name.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_support::scratch_dir;

    #[test]
    fn a_fresh_folder_is_fully_accessible_to_its_owner() {
        let dir = scratch_dir("access-owner");

        let access = effective_access(&dir);

        assert!(access.read && access.write && access.execute);
        assert!(can_write(&dir));
        assert_eq!(access.describe(), "read, write, execute");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_path_is_not_accessible() {
        let missing = Path::new("/definitely/not/here");

        assert_eq!(effective_access(missing).describe(), "no access");
        assert!(!can_write(missing));
    }

    #[test]
    fn describe_reads_naturally() {
        let read_only = Access {
            read: true,
            write: false,
            execute: false,
        };

        assert_eq!(read_only.describe(), "read only");
    }

    #[test]
    fn names_are_found_by_numeric_id_and_bad_lines_are_skipped() {
        let passwd = "root:x:0:0:root:/root:/bin/bash\nbroken line\nshark:x:1000:1000::/home/shark:/bin/sh\n";

        assert_eq!(lookup_in(passwd, 0).as_deref(), Some("root"));
        assert_eq!(lookup_in(passwd, 1000).as_deref(), Some("shark"));
        assert_eq!(lookup_in(passwd, 4242), None);
    }
}
