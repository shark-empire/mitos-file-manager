//! Network locations the user has connected to before ("Connect to
//! Server..."), remembered so they can be picked again with one click.
//! Stored next to the bookmarks; never with a password in it.

use crate::navigation::bookmarks::config_dir;
use std::fs;
use std::path::PathBuf;

/// How many servers are remembered.
const MAX_SERVERS: usize = 8;

fn servers_file() -> PathBuf {
    config_dir().join("servers.json")
}

/// Recently used server addresses, newest first.
pub fn load() -> Vec<String> {
    fs::read_to_string(servers_file())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Remember that `uri` was connected to (moving it to the front if it was
/// already there).
pub fn remember(uri: &str) {
    let mut servers = load();

    push_recent(&mut servers, uri);

    let dir = config_dir();
    let _ = fs::create_dir_all(&dir);

    if let Ok(json) = serde_json::to_string_pretty(&servers) {
        let _ = fs::write(servers_file(), json);
    }
}

/// Put `uri` at the front of `servers` -- without any password embedded in
/// it, and without duplicates -- and keep the list to `MAX_SERVERS`.
fn push_recent(servers: &mut Vec<String>, uri: &str) {
    let uri = strip_password(uri.trim());

    if uri.is_empty() {
        return;
    }

    servers.retain(|existing| *existing != uri);
    servers.insert(0, uri);
    servers.truncate(MAX_SERVERS);
}

/// `smb://user:secret@host/share` -> `smb://user@host/share`. Credentials
/// don't belong in a plain-text file in the config folder; the connection
/// prompts for the password again (and GVfs can keep it in the keyring).
fn strip_password(uri: &str) -> String {
    let Some(scheme_end) = uri.find("://") else {
        return uri.to_string();
    };

    let authority_start = scheme_end + 3;
    let rest = &uri[authority_start..];
    let authority_len = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_len];

    let Some(at) = authority.rfind('@') else {
        return uri.to_string();
    };

    let userinfo = &authority[..at];

    let Some(colon) = userinfo.find(':') else {
        return uri.to_string();
    };

    format!(
        "{}{}{}",
        &uri[..authority_start],
        &authority[..colon],
        &uri[authority_start + at..]
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_are_removed_but_the_user_name_stays() {
        assert_eq!(
            strip_password("smb://user:secret@server/share"),
            "smb://user@server/share"
        );
        assert_eq!(
            strip_password("sftp://me:p%40ss@host:2222/home/me"),
            "sftp://me@host:2222/home/me"
        );
    }

    #[test]
    fn addresses_without_a_password_are_left_alone() {
        for uri in [
            "smb://server/share",
            "sftp://me@host/path",
            "ftp://host",
            "not a uri",
        ] {
            assert_eq!(strip_password(uri), uri);
        }
    }

    #[test]
    fn a_colon_after_the_host_is_a_port_not_a_password() {
        assert_eq!(strip_password("ftp://host:21/dir"), "ftp://host:21/dir");
    }

    #[test]
    fn recent_servers_are_deduplicated_newest_first_and_capped() {
        let mut servers = Vec::new();

        push_recent(&mut servers, "smb://a/x");
        push_recent(&mut servers, "smb://b/x");
        push_recent(&mut servers, "smb://a/x");

        assert_eq!(servers, vec!["smb://a/x", "smb://b/x"]);

        for i in 0..20 {
            push_recent(&mut servers, &format!("smb://host{i}/x"));
        }

        assert_eq!(servers.len(), MAX_SERVERS);
        assert_eq!(servers[0], "smb://host19/x");

        push_recent(&mut servers, "   ");
        assert_eq!(servers.len(), MAX_SERVERS);
    }
}
