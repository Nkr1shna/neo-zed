use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use gpui::App;
use plugins_ui::{
    PluginPanelRecord, PluginRecord, PluginSource, PluginStatus, PluginStoreApi, PluginsPage,
};
use ui::prelude::SharedString;

#[derive(Clone, Debug, PartialEq, Eq)]
enum StoreCall {
    Install(Arc<str>),
    Remove(Arc<str>),
    InstallDevelopment(PathBuf),
    OpenPanel {
        plugin_id: Arc<str>,
        panel_id: Arc<str>,
    },
}

struct MockPluginStore {
    records: Mutex<Vec<PluginRecord>>,
    calls: Mutex<Vec<StoreCall>>,
}

impl MockPluginStore {
    fn new(records: Vec<PluginRecord>) -> Self {
        Self {
            records: Mutex::new(records),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<StoreCall> {
        self.calls
            .lock()
            .expect("store call log lock poisoned")
            .clone()
    }

    fn plugin_status(&self, plugin_id: &str) -> Option<PluginStatus> {
        self.records
            .lock()
            .expect("store records lock poisoned")
            .iter()
            .find(|record| record.id.as_ref() == plugin_id)
            .map(|record| record.status.clone())
    }
}

impl PluginStoreApi for MockPluginStore {
    fn list_plugins(&self) -> Result<Vec<PluginRecord>> {
        Ok(self
            .records
            .lock()
            .expect("store records lock poisoned")
            .clone())
    }

    fn install_plugin(&self, plugin_id: &str, _cx: &mut App) -> Result<()> {
        self.calls
            .lock()
            .expect("store call log lock poisoned")
            .push(StoreCall::Install(Arc::from(plugin_id)));
        let mut records = self.records.lock().expect("store records lock poisoned");
        let record = records
            .iter_mut()
            .find(|record| record.id.as_ref() == plugin_id)
            .ok_or_else(|| anyhow!("missing plugin: {plugin_id}"))?;
        record.status = PluginStatus::Installed;
        Ok(())
    }

    fn remove_plugin(&self, plugin_id: &str, _cx: &mut App) -> Result<()> {
        self.calls
            .lock()
            .expect("store call log lock poisoned")
            .push(StoreCall::Remove(Arc::from(plugin_id)));
        let mut records = self.records.lock().expect("store records lock poisoned");
        let record = records
            .iter_mut()
            .find(|record| record.id.as_ref() == plugin_id)
            .ok_or_else(|| anyhow!("missing plugin: {plugin_id}"))?;
        record.status = PluginStatus::NotInstalled;
        Ok(())
    }

    fn install_development_plugin(
        &self,
        source_directory: &std::path::Path,
        _cx: &mut App,
    ) -> Result<()> {
        self.calls
            .lock()
            .expect("store call log lock poisoned")
            .push(StoreCall::InstallDevelopment(
                source_directory.to_path_buf(),
            ));
        let mut records = self.records.lock().expect("store records lock poisoned");
        let record = records
            .iter_mut()
            .find(|record| {
                record.source.development_path().map(PathBuf::as_path) == Some(source_directory)
            })
            .ok_or_else(|| anyhow!("missing plugin: {}", source_directory.display()))?;
        record.status = PluginStatus::Installed;
        Ok(())
    }

    fn open_panel(
        &self,
        plugin_id: &str,
        panel_id: &str,
        _window: &mut gpui::Window,
        _cx: &mut App,
    ) -> Result<()> {
        self.calls
            .lock()
            .expect("store call log lock poisoned")
            .push(StoreCall::OpenPanel {
                plugin_id: Arc::from(plugin_id),
                panel_id: Arc::from(panel_id),
            });
        Ok(())
    }
}

fn registry_plugin(id: &str, status: PluginStatus) -> PluginRecord {
    PluginRecord {
        id: Arc::from(id),
        name: SharedString::from(format!("Registry {id}")),
        version: SharedString::from("1.0.0"),
        description: Some(SharedString::from("Registry plugin")),
        authors: vec!["Registry Team".into()],
        repository_url: Some("https://github.com/example/registry-plugin".into()),
        homepage_url: Some("https://example.com/registry-plugin".into()),
        source: PluginSource::Registry,
        status,
        panels: vec![PluginPanelRecord {
            id: Arc::from(format!("{id}-panel")),
            title: SharedString::from("Registry Panel"),
        }],
    }
}

fn development_plugin(id: &str, status: PluginStatus) -> PluginRecord {
    PluginRecord {
        id: Arc::from(id),
        name: SharedString::from(format!("Dev {id}")),
        version: SharedString::from("0.1.0"),
        description: Some(SharedString::from("Development plugin")),
        authors: vec!["Development Team".into()],
        repository_url: Some("https://github.com/example/dev-plugin".into()),
        homepage_url: Some("https://example.com/dev-plugin".into()),
        source: PluginSource::Development {
            path: PathBuf::from(format!("/tmp/{id}")),
        },
        status,
        panels: vec![PluginPanelRecord {
            id: Arc::from(format!("{id}-panel")),
            title: SharedString::from("Dev Panel"),
        }],
    }
}

#[test]
fn plugin_records_expose_state_and_development_path() {
    let record = development_plugin("alpha", PluginStatus::NotInstalled);

    assert_eq!(
        record.source.label(),
        SharedString::from("Development: /tmp/alpha")
    );
    assert_eq!(record.action_label(), SharedString::from("Install Dev"));
    assert_eq!(record.status.label(), SharedString::from("Not installed"));
    assert_eq!(
        record.source.development_path().map(PathBuf::from),
        Some(PathBuf::from("/tmp/alpha"))
    );
    assert_eq!(record.panels.len(), 1);
    assert_eq!(record.panels[0].title, SharedString::from("Dev Panel"));
}

#[test]
fn page_refreshes_and_tracks_install_remove_transitions() {
    let store = Arc::new(MockPluginStore::new(vec![
        registry_plugin("registry-alpha", PluginStatus::NotInstalled),
        development_plugin("dev-beta", PluginStatus::NotInstalled),
    ]));

    let mut page = PluginsPage::new(store.clone());
    page.reload_from_store()
        .expect("page should load plugins from the store");
    let mut app = gpui::TestApp::new();

    assert_eq!(
        page.plugin_records()
            .iter()
            .map(|record| record.action_label())
            .collect::<Vec<_>>(),
        vec![
            SharedString::from("Install Dev"),
            SharedString::from("Install")
        ]
    );

    app.update(|cx| page.install_plugin("registry-alpha", cx))
        .expect("registry plugin install should succeed");
    assert_eq!(
        store.plugin_status("registry-alpha"),
        Some(PluginStatus::Installed)
    );
    assert_eq!(
        page.plugin_records()
            .iter()
            .find(|record| record.id.as_ref() == "registry-alpha")
            .map(|record| record.action_label()),
        Some(SharedString::from("Remove"))
    );

    app.update(|cx| page.remove_plugin("registry-alpha", cx))
        .expect("registry plugin removal should succeed");
    assert_eq!(
        store.plugin_status("registry-alpha"),
        Some(PluginStatus::NotInstalled)
    );

    app.update(|cx| page.install_development_plugin(std::path::Path::new("/tmp/dev-beta"), cx))
        .expect("development plugin install should succeed");
    assert_eq!(
        store.plugin_status("dev-beta"),
        Some(PluginStatus::Installed)
    );
    assert_eq!(
        page.plugin_records()
            .iter()
            .find(|record| record.id.as_ref() == "dev-beta")
            .map(|record| record.action_label()),
        Some(SharedString::from("Remove"))
    );

    assert_eq!(
        store.calls(),
        vec![
            StoreCall::Install(Arc::from("registry-alpha")),
            StoreCall::Remove(Arc::from("registry-alpha")),
            StoreCall::InstallDevelopment(PathBuf::from("/tmp/dev-beta")),
        ]
    );
}

#[test]
fn focus_plugin_tracks_selected_identifier() {
    let store = Arc::new(MockPluginStore::new(vec![]));
    let mut page = PluginsPage::new(store);

    assert_eq!(page.focused_plugin_id(), None);

    page.focus_plugin("test-plugin");

    assert_eq!(page.focused_plugin_id(), Some("test-plugin"));
}
