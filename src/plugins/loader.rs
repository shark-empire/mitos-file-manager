use crate::plugins::manifest::PluginManifest;
use std::fs;
use std::path::PathBuf;

pub fn plugin_directory() -> Option<PathBuf> {
    let config = dirs::config_dir()?;
    Some(config.join("mitos/file-manager/plugins"))
}

pub fn load_plugins() -> Vec<(PathBuf, PluginManifest)> {
    let Some(dir) = plugin_directory() else {
        return Vec::new();
    };

    let mut plugins = Vec::new();

    if !dir.exists() {
        return plugins;
    }

    let Ok(entries) = fs::read_dir(&dir) else {
        return plugins;
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            let manifest_path = path.join("plugin.json");

            if manifest_path.exists() {
                if let Ok(content) = fs::read_to_string(&manifest_path) {
                    if let Ok(manifest) = serde_json::from_str::<PluginManifest>(&content) {
                        plugins.push((path, manifest));
                    }
                }
            }
        }
    }

    plugins
}

/// Wrap `text` in single quotes for `sh`, escaping any single quote inside
/// it, so it reaches the command as exactly one literal argument.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

pub fn execute_plugin_action(command: &str, file_paths: &[PathBuf]) -> Result<(), String> {
    // The command runs through `sh -c`, so a file called `a b` or `x; rm -rf ~`
    // pasted in raw would split into several arguments -- or run as a
    // command. Quoting each path makes `{files}` mean "these files" and
    // nothing else.
    let paths_str: Vec<String> = file_paths
        .iter()
        .map(|p| shell_quote(&p.display().to_string()))
        .collect();

    let full_command = command.replace("{files}", &paths_str.join(" "));

    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(&full_command)
        .spawn()
        .map_err(|e| format!("Failed to execute plugin command: {}", e))?;

    // Reap the child when it exits; an un-waited process lingers as a
    // zombie until this app quits.
    std::thread::spawn(move || {
        let _ = child.wait();
    });

    Ok(())
}
