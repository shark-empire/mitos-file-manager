//! What is mounted where, read straight from `/proc` -- no GTK, no D-Bus,
//! so it is safe to call from any thread (the trash service and the
//! protected-path checks both do).

use std::fs;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct MountPoint {
    pub device: String,
    pub path: PathBuf,
    pub fs_type: String,
}

/// Every mount the kernel currently reports for this process.
pub fn list() -> Vec<MountPoint> {
    fs::read_to_string("/proc/self/mounts")
        .map(|text| parse(&text))
        .unwrap_or_default()
}

/// Parse the text of `/proc/self/mounts`: `device mountpoint fstype opts 0 0`.
pub fn parse(text: &str) -> Vec<MountPoint> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();

            let device = fields.next()?;
            let path = fields.next()?;
            let fs_type = fields.next()?;

            Some(MountPoint {
                device: unescape(device),
                path: PathBuf::from(unescape(path)),
                fs_type: fs_type.to_string(),
            })
        })
        .collect()
}

/// `/proc/mounts` writes a space, tab, newline or backslash inside a field
/// as a three-digit octal escape (`\040`, `\011`, `\012`, `\134`).
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 3 < bytes.len() {
            let digits = &bytes[i + 1..i + 4];

            if digits.iter().all(|d| (b'0'..=b'7').contains(d)) && digits[0] <= b'3' {
                let value = (digits[0] - b'0') * 64 + (digits[1] - b'0') * 8 + (digits[2] - b'0');
                out.push(value);
                i += 4;
                continue;
            }
        }

        out.push(bytes[i]);
        i += 1;
    }

    String::from_utf8_lossy(&out).into_owned()
}

/// Filesystems that only exist in memory or in the kernel: nothing on them
/// is a user's, and none has a trash can.
const PSEUDO_FILESYSTEMS: &[&str] = &[
    "autofs",
    "binfmt_misc",
    "bpf",
    "cgroup",
    "cgroup2",
    "configfs",
    "debugfs",
    "devpts",
    "devtmpfs",
    "efivarfs",
    "fusectl",
    "hugetlbfs",
    "mqueue",
    "nsfs",
    "overlay",
    "proc",
    "pstore",
    "ramfs",
    "rpc_pipefs",
    "securityfs",
    "squashfs",
    "sysfs",
    "tmpfs",
    "tracefs",
];

/// Removable media and network shares -- the real filesystems mounted under
/// `/media`, `/run/media` or `/mnt`, which can carry their own per-device
/// trash can -- as `(display name, mount path)` pairs.
pub fn removable_roots() -> Vec<(String, PathBuf)> {
    roots_from(list())
}

fn roots_from(mounts: Vec<MountPoint>) -> Vec<(String, PathBuf)> {
    mounts
        .into_iter()
        .filter(|mount| {
            let under_a_media_folder = mount.path.starts_with("/media")
                || mount.path.starts_with("/run/media")
                || mount.path.starts_with("/mnt");

            under_a_media_folder && !PSEUDO_FILESYSTEMS.contains(&mount.fs_type.as_str())
        })
        .map(|mount| {
            // The mount's folder name ("STICK"); the device it came from
            // only if the path has no name of its own.
            let label = mount
                .path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| mount.device.clone());

            (label, mount.path)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
sysfs /sys sysfs rw,nosuid,nodev,noexec,relatime 0 0
/dev/sda2 / ext4 rw,relatime 0 0
/dev/sdb1 /media/user/My\\040Stick vfat rw,nosuid 0 0
//nas/share /mnt/tab\\011dir cifs rw 0 0
";

    #[test]
    fn parses_device_path_and_type() {
        let mounts = parse(SAMPLE);

        assert_eq!(mounts.len(), 4);
        assert_eq!(mounts[1].device, "/dev/sda2");
        assert_eq!(mounts[1].path, PathBuf::from("/"));
        assert_eq!(mounts[1].fs_type, "ext4");
    }

    #[test]
    fn octal_escapes_in_paths_are_decoded() {
        let mounts = parse(SAMPLE);

        assert_eq!(mounts[2].path, PathBuf::from("/media/user/My Stick"));
        assert_eq!(mounts[3].path, PathBuf::from("/mnt/tab\tdir"));
    }

    #[test]
    fn removable_roots_are_real_filesystems_under_the_media_folders() {
        let mounts = parse(
            "/dev/sda2 / ext4 rw 0 0\n\
             /dev/sdb1 /media/user/STICK vfat rw 0 0\n\
             tmpfs /run/media/scratch tmpfs rw 0 0\n\
             //nas/share /mnt/nas cifs rw 0 0\n",
        );

        assert_eq!(
            roots_from(mounts),
            vec![
                ("STICK".to_string(), PathBuf::from("/media/user/STICK")),
                ("nas".to_string(), PathBuf::from("/mnt/nas")),
            ]
        );
    }

    #[test]
    fn short_or_blank_lines_are_ignored() {
        assert!(parse("\n   \nonlyone\ntwo fields\n").is_empty());
    }

    #[test]
    fn a_backslash_that_is_not_an_escape_survives() {
        assert_eq!(unescape("a\\b"), "a\\b");
        assert_eq!(unescape("trailing\\"), "trailing\\");
        assert_eq!(unescape("\\999"), "\\999");
    }
}
