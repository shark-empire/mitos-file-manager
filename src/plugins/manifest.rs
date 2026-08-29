use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub actions: Vec<PluginAction>,
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
