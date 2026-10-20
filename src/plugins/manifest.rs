use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub actions: Vec<PluginAction>,
}

impl PluginManifest {
    /// Hover text for this plugin's menu entries: what it does, then who it
    /// is from ("Open Terminal 1.0.0 -- MITOS").
    pub fn tooltip(&self) -> String {
        let credit = format!("{} {} \u{2014} {}", self.name, self.version, self.author);
        let description = self.description.trim();

        if description.is_empty() {
            credit
        } else {
            format!("{description}\n{credit}")
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PluginAction {
    pub id: String,
    pub label: String,
    pub command: String,
    pub applies_to: AppliesTo,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AppliesTo {
    Files,
    Folders,
    Both,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(description: &str) -> PluginManifest {
        PluginManifest {
            name: "Open Terminal".to_string(),
            version: "1.0.0".to_string(),
            description: description.to_string(),
            author: "MITOS".to_string(),
            actions: Vec::new(),
        }
    }

    #[test]
    fn the_tooltip_says_what_it_does_and_who_it_is_from() {
        assert_eq!(
            manifest("Opens a terminal").tooltip(),
            "Opens a terminal\nOpen Terminal 1.0.0 \u{2014} MITOS"
        );
    }

    #[test]
    fn without_a_description_the_tooltip_is_just_the_credit() {
        assert_eq!(
            manifest("   ").tooltip(),
            "Open Terminal 1.0.0 \u{2014} MITOS"
        );
    }

    #[test]
    fn a_plugin_json_parses_with_its_actions() {
        let json = r#"{
            "name": "Open Terminal",
            "version": "1.0.0",
            "description": "Opens a terminal in the selected folder",
            "author": "MITOS",
            "actions": [{
                "id": "open_terminal",
                "label": "Open Terminal Here",
                "command": "cd {files} && x-terminal-emulator",
                "applies_to": "folders"
            }]
        }"#;

        let parsed: PluginManifest = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.actions.len(), 1);
        assert_eq!(parsed.actions[0].id, "open_terminal");
        assert_eq!(parsed.actions[0].applies_to, AppliesTo::Folders);
    }
}
