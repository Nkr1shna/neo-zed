use collections::HashMap;
use settings::{RegisterSetting, Settings};
use std::sync::Arc;

#[derive(Debug, Default, Clone, RegisterSetting)]
pub struct PluginSettings {
    /// Plugins that should be installed automatically.
    pub auto_install_plugins: HashMap<Arc<str>, bool>,
    pub auto_update_plugins: HashMap<Arc<str>, bool>,
}

impl PluginSettings {
    pub fn should_auto_install(&self, plugin_id: &str) -> bool {
        self.auto_install_plugins
            .get(plugin_id)
            .copied()
            .unwrap_or(false)
    }

    pub fn should_auto_update(&self, plugin_id: &str) -> bool {
        self.auto_update_plugins
            .get(plugin_id)
            .copied()
            .unwrap_or(true)
    }
}

impl Settings for PluginSettings {
    fn from_settings(content: &settings::SettingsContent) -> Self {
        Self {
            auto_install_plugins: content.plugin.auto_install_plugins.clone(),
            auto_update_plugins: content.plugin.auto_update_plugins.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PluginSettings;
    use gpui::{App, UpdateGlobal as _};
    use settings::{Settings, SettingsStore};

    #[gpui::test]
    fn plugin_settings_default_to_empty_install_and_enabled_updates(cx: &mut App) {
        let store = SettingsStore::test(cx);
        cx.set_global(store);

        let settings = PluginSettings::get_global(cx);
        assert!(settings.auto_install_plugins.is_empty());
        assert!(settings.auto_update_plugins.is_empty());
        assert!(!settings.should_auto_install("typescript"));
        assert!(settings.should_auto_update("typescript"));
    }

    #[gpui::test]
    fn plugin_settings_follow_user_settings(cx: &mut App) {
        let store = SettingsStore::test(cx);
        cx.set_global(store);

        SettingsStore::update_global(cx, |store, cx| {
            store
                .set_user_settings(
                    r#"{
                        "auto_install_plugins": {
                            "typescript": true
                        },
                        "auto_update_plugins": {
                            "typescript": false
                        }
                    }"#,
                    cx,
                )
                .unwrap();
        });

        let settings = PluginSettings::get_global(cx);
        assert!(settings.should_auto_install("typescript"));
        assert!(!settings.should_auto_update("typescript"));
        assert!(!settings.should_auto_install("python"));
        assert!(settings.should_auto_update("python"));
    }
}
