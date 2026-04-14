use std::sync::Arc;

use collections::HashMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings_macros::{MergeFrom, with_fallible_options};

#[with_fallible_options]
#[derive(Debug, PartialEq, Clone, Default, Serialize, Deserialize, JsonSchema, MergeFrom)]
pub struct PluginSettingsContent {
    /// The plugins that should be installed automatically by Neo Zed.
    ///
    /// Default: {}
    #[serde(default)]
    pub auto_install_plugins: HashMap<Arc<str>, bool>,
    /// Whether installed registry plugins should update automatically.
    ///
    /// Default: enabled unless a plugin is explicitly set to `false`.
    #[serde(default)]
    pub auto_update_plugins: HashMap<Arc<str>, bool>,
}

#[cfg(test)]
mod tests {
    use super::PluginSettingsContent;

    #[test]
    fn defaults_are_empty() {
        let settings: PluginSettingsContent = serde_json::from_str("{}").unwrap();

        assert!(settings.auto_install_plugins.is_empty());
        assert!(settings.auto_update_plugins.is_empty());
    }

    #[test]
    fn parses_explicit_values() {
        let settings: PluginSettingsContent = serde_json::from_str(
            r#"{
                "auto_install_plugins": {
                    "typescript": false
                },
                "auto_update_plugins": {
                    "typescript": true
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            settings.auto_install_plugins.get("typescript").copied(),
            Some(false)
        );
        assert_eq!(
            settings.auto_update_plugins.get("typescript").copied(),
            Some(true)
        );
    }
}
