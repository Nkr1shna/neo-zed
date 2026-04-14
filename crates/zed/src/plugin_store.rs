use anyhow::{Context as _, Result};
use client::Client;
use cloud_api_types::{GetPluginsResponse, PluginMetadata as CloudPluginMetadata};
use collections::HashMap;
use futures::AsyncReadExt as _;
use gpui::{App, AppContext, Task, WeakEntity};
use plugin::PluginState;
use plugins_ui::{
    PluginPanelRecord, PluginRecord, PluginSource, PluginStatus, PluginStoreApi,
    PluginStoreProvider,
};
use settings::Settings;
use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use crate::plugin_settings::PluginSettings;

pub(crate) fn init(client: Arc<Client>, cx: &mut App) {
    let layout = plugin_store_layout();
    plugins_ui::init_with_provider(
        Arc::new(ZedPluginStoreProvider {
            layout: layout.clone(),
            client: client.clone(),
        }),
        cx,
    );
    sync_plugins_with_registry(client, layout, cx);
}

struct ZedPluginStoreProvider {
    layout: plugin::PluginStoreLayout,
    client: Arc<Client>,
}

impl PluginStoreProvider for ZedPluginStoreProvider {
    fn plugin_store(&self, workspace: &workspace::Workspace, _cx: &App) -> Arc<dyn PluginStoreApi> {
        Arc::new(ZedPluginStore {
            layout: self.layout.clone(),
            client: self.client.clone(),
            workspace: workspace.weak_handle(),
        })
    }
}

struct ZedPluginStore {
    layout: plugin::PluginStoreLayout,
    client: Arc<Client>,
    workspace: WeakEntity<workspace::Workspace>,
}

impl ZedPluginStore {
    fn map_plugin_record(
        layout: &plugin::PluginStoreLayout,
        plugin: plugin::InstalledPlugin,
    ) -> PluginRecord {
        let source = infer_plugin_source(layout, &plugin);

        let status = match (plugin.state, plugin.error_message.as_deref()) {
            (_, Some(message)) => PluginStatus::Failed(message.to_string().into()),
            (PluginState::Error, None) => PluginStatus::Failed("Plugin error".into()),
            (PluginState::Installed | PluginState::Development, None) => PluginStatus::Installed,
        };

        let panels = plugin
            .manifest
            .panels
            .iter()
            .map(|panel| PluginPanelRecord {
                id: Arc::from(panel.id.clone()),
                title: panel.title.clone().into(),
            })
            .collect();

        PluginRecord {
            id: Arc::from(plugin.manifest.id.as_str()),
            name: plugin.manifest.name.into(),
            version: plugin.manifest.version.into(),
            description: plugin.manifest.description.map(Into::into),
            authors: plugin
                .manifest
                .authors
                .into_iter()
                .map(Into::into)
                .collect(),
            repository_url: plugin.manifest.repository.map(Into::into),
            homepage_url: plugin.manifest.homepage.map(Into::into),
            source,
            status,
            panels,
        }
    }

    fn map_remote_plugin_record(
        plugin: CloudPluginMetadata,
        status: PluginStatus,
        source: PluginSource,
    ) -> PluginRecord {
        PluginRecord {
            id: plugin.id.clone(),
            name: plugin.manifest.name.into(),
            version: plugin.manifest.version.to_string().into(),
            description: plugin.manifest.description.map(Into::into),
            authors: plugin
                .manifest
                .authors
                .into_iter()
                .map(Into::into)
                .collect(),
            repository_url: plugin.manifest.repository.map(Into::into),
            homepage_url: plugin.manifest.homepage.map(Into::into),
            source,
            status,
            panels: plugin
                .manifest
                .panels
                .into_iter()
                .map(|panel| PluginPanelRecord {
                    id: Arc::from(panel.id),
                    title: panel.title.into(),
                })
                .collect(),
        }
    }

    async fn fetch_remote_plugins(client: Arc<Client>) -> Result<Vec<CloudPluginMetadata>> {
        let operating_system = std::env::consts::OS.to_string();
        let architecture = std::env::consts::ARCH.to_string();
        let url = client.http_client().build_zed_api_url(
            "/plugins",
            &[
                ("max_schema_version", "1"),
                ("os", operating_system.as_str()),
                ("arch", architecture.as_str()),
                ("allow_source_fallback", "true"),
            ],
        )?;

        let mut response = client
            .http_client()
            .get(url.as_ref(), Default::default(), true)
            .await
            .context("fetching plugins")?;
        let mut body = Vec::new();
        response
            .body_mut()
            .read_to_end(&mut body)
            .await
            .context("reading plugins response")?;
        if response.status().is_client_error() || response.status().is_server_error() {
            anyhow::bail!(
                "plugin registry request failed with status {}",
                response.status().as_u16()
            );
        }

        let response: GetPluginsResponse = serde_json::from_slice(&body)?;
        Ok(response.data)
    }

    async fn load_plugin_records(
        layout: plugin::PluginStoreLayout,
        client: Arc<Client>,
    ) -> Result<Vec<PluginRecord>> {
        let local_plugins = plugin::PluginStore::new(layout.clone()).list()?;
        let remote_plugins = match Self::fetch_remote_plugins(client).await {
            Ok(remote_plugins) => remote_plugins,
            Err(error) if local_plugins.is_empty() => return Err(error),
            Err(error) => {
                log::error!("failed to fetch remote plugins: {error:#}");
                Vec::new()
            }
        };

        let mut local_plugins_by_id = local_plugins
            .into_iter()
            .map(|plugin| (plugin.manifest.id.as_str().to_string(), plugin))
            .collect::<HashMap<_, _>>();

        let mut plugins = Vec::new();
        for remote_plugin in remote_plugins {
            let plugin_id = remote_plugin.id.to_string();
            if let Some(local_plugin) = local_plugins_by_id.remove(&plugin_id) {
                match local_plugin.state {
                    PluginState::Development => {
                        plugins.push(Self::map_plugin_record(&layout, local_plugin));
                    }
                    PluginState::Installed => {
                        if local_plugin.manifest.version
                            != remote_plugin.manifest.version.to_string()
                        {
                            let mut plugin = Self::map_plugin_record(&layout, local_plugin.clone());
                            plugin.source = PluginSource::Registry;
                            plugin.status = PluginStatus::UpdateAvailable {
                                installed_version: local_plugin.manifest.version.into(),
                                latest_version: remote_plugin.manifest.version.to_string().into(),
                            };
                            plugins.push(plugin);
                        } else {
                            plugins.push(Self::map_remote_plugin_record(
                                remote_plugin,
                                PluginStatus::Installed,
                                PluginSource::Registry,
                            ));
                        }
                    }
                    PluginState::Error => {
                        plugins.push(Self::map_plugin_record(&layout, local_plugin));
                    }
                }
            } else {
                plugins.push(Self::map_remote_plugin_record(
                    remote_plugin,
                    PluginStatus::NotInstalled,
                    PluginSource::Registry,
                ));
            }
        }

        plugins.extend(
            local_plugins_by_id
                .into_values()
                .map(|plugin| Self::map_plugin_record(&layout, plugin)),
        );
        Ok(plugins)
    }

    async fn install_registry_plugin(
        layout: plugin::PluginStoreLayout,
        client: Arc<Client>,
        plugin_id: String,
    ) -> Result<()> {
        let operating_system = std::env::consts::OS.to_string();
        let architecture = std::env::consts::ARCH.to_string();
        let url = client.http_client().build_zed_api_url(
            &format!("/plugins/{plugin_id}/download"),
            &[
                ("os", operating_system.as_str()),
                ("arch", architecture.as_str()),
                ("allow_source_fallback", "true"),
            ],
        )?;

        let mut response = client
            .http_client()
            .get(url.as_ref(), Default::default(), true)
            .await
            .with_context(|| format!("downloading plugin `{plugin_id}`"))?;
        let status = response.status();
        let mut archive_bytes = Vec::new();
        response
            .body_mut()
            .read_to_end(&mut archive_bytes)
            .await
            .context("reading plugin archive")?;
        if !status.is_success() {
            let response_text = String::from_utf8_lossy(&archive_bytes);
            anyhow::bail!(
                "plugin download failed with status {}: {response_text}",
                status.as_u16()
            );
        }

        let mut store = plugin::PluginStore::new(layout);
        store
            .install_registry_plugin_from_tar_gz(archive_bytes.as_slice())
            .await?;
        Ok(())
    }
}

fn infer_plugin_source(
    layout: &plugin::PluginStoreLayout,
    plugin: &plugin::InstalledPlugin,
) -> PluginSource {
    match plugin.state {
        PluginState::Development => PluginSource::Development {
            path: plugin.installation.root.clone(),
        },
        PluginState::Installed => PluginSource::Registry,
        PluginState::Error => {
            if plugin.installation.root.starts_with(&layout.installed_root) {
                PluginSource::Registry
            } else {
                PluginSource::Development {
                    path: plugin.installation.root.clone(),
                }
            }
        }
    }
}

impl PluginStoreApi for ZedPluginStore {
    fn list_plugins(&self, cx: &mut App) -> Task<Result<Vec<PluginRecord>>> {
        let layout = self.layout.clone();
        let client = self.client.clone();
        cx.background_spawn(async move { Self::load_plugin_records(layout, client).await })
    }

    fn install_plugin(&self, plugin_id: &str, cx: &mut App) -> Task<Result<()>> {
        let layout = self.layout.clone();
        let client = self.client.clone();
        let plugin_id = plugin_id.to_string();
        cx.spawn(async move |cx| {
            let result: Result<()> = cx
                .background_spawn(async move {
                    Self::install_registry_plugin(layout, client, plugin_id).await
                })
                .await;
            if result.is_ok() {
                cx.update(|cx| plugin_host::refresh_catalog(cx));
            }
            result
        })
    }

    fn remove_plugin(&self, plugin_id: &str, cx: &mut App) -> Task<Result<()>> {
        let layout = self.layout.clone();
        let plugin_id = plugin_id.to_string();
        cx.spawn(async move |cx| {
            let result: Result<()> = cx
                .background_spawn(async move {
                    let mut store = plugin::PluginStore::new(layout);
                    store.remove(&plugin_id)?;
                    Ok(())
                })
                .await;
            if result.is_ok() {
                cx.update(|cx| plugin_host::refresh_catalog(cx));
            }
            result
        })
    }

    fn install_development_plugin(
        &self,
        source_directory: &Path,
        cx: &mut App,
    ) -> Task<Result<()>> {
        let layout = self.layout.clone();
        let source_directory = source_directory.to_path_buf();
        cx.spawn(async move |cx| {
            let result: Result<()> = cx
                .background_spawn(async move {
                    let mut store = plugin::PluginStore::new(layout);
                    store.register_development_plugin(source_directory)?;
                    Ok(())
                })
                .await;
            if result.is_ok() {
                cx.update(|cx| plugin_host::refresh_catalog(cx));
            }
            result
        })
    }

    fn open_panel(
        &self,
        plugin_id: &str,
        panel_id: &str,
        window: &mut gpui::Window,
        cx: &mut App,
    ) -> Result<()> {
        let workspace = self
            .workspace
            .upgrade()
            .context("workspace is no longer available")?;
        workspace.update(cx, |workspace, workspace_cx| {
            plugin_host::open_panel_in_workspace(
                plugin_id,
                panel_id,
                workspace,
                window,
                workspace_cx,
            )?;
            Ok::<(), anyhow::Error>(())
        })?;
        Ok(())
    }
}

fn plugin_store_layout() -> plugin::PluginStoreLayout {
    let root = paths::plugins_dir();
    plugin::PluginStoreLayout {
        installed_root: root.join("installed"),
        development_root: root.join("development"),
    }
}

fn sync_plugins_with_registry(
    client: Arc<Client>,
    layout: plugin::PluginStoreLayout,
    cx: &mut App,
) {
    if cfg!(test) {
        return;
    }

    let plugin_settings = PluginSettings::get_global(cx).clone();

    cx.spawn(async move |cx| {
        let result: Result<bool> = cx
            .background_spawn(async move {
                let local_plugins = plugin::PluginStore::new(layout.clone()).list()?;
                let mut development_plugin_ids = BTreeSet::new();
                let mut installed_registry_versions = HashMap::default();
                for plugin in local_plugins {
                    let plugin_id = plugin.manifest.id.as_str().to_string();
                    match plugin.state {
                        PluginState::Development => {
                            development_plugin_ids.insert(plugin_id);
                        }
                        PluginState::Installed => {
                            if matches!(
                                plugin.installation.source,
                                plugin::PluginInstallSource::Registry { .. }
                            ) {
                                installed_registry_versions
                                    .insert(plugin_id, plugin.manifest.version.clone());
                            }
                        }
                        PluginState::Error => {}
                    }
                }

                let mut plugin_ids_to_install = plugin_settings
                    .auto_install_plugins
                    .iter()
                    .filter_map(|(plugin_id, _)| {
                        if !plugin_settings.should_auto_install(plugin_id.as_ref()) {
                            return None;
                        }

                        let plugin_id = plugin_id.to_string();
                        if development_plugin_ids.contains(plugin_id.as_str())
                            || installed_registry_versions.contains_key(plugin_id.as_str())
                        {
                            None
                        } else {
                            Some(plugin_id)
                        }
                    })
                    .collect::<Vec<_>>();

                if plugin_ids_to_install.is_empty() && installed_registry_versions.is_empty() {
                    return Ok(false);
                }

                let remote_plugins = ZedPluginStore::fetch_remote_plugins(client.clone()).await?;
                let remote_plugins_by_id = remote_plugins
                    .into_iter()
                    .map(|plugin| (plugin.id.to_string(), plugin))
                    .collect::<HashMap<_, _>>();

                plugin_ids_to_install
                    .retain(|plugin_id| remote_plugins_by_id.contains_key(plugin_id));

                for (plugin_id, installed_version) in installed_registry_versions.iter() {
                    if development_plugin_ids.contains(plugin_id) {
                        continue;
                    }
                    if !plugin_settings.should_auto_update(plugin_id) {
                        continue;
                    }

                    let Some(remote_plugin) = remote_plugins_by_id.get(plugin_id) else {
                        continue;
                    };
                    if installed_version != &remote_plugin.manifest.version.to_string() {
                        plugin_ids_to_install.push(plugin_id.clone());
                    }
                }

                plugin_ids_to_install.sort();
                plugin_ids_to_install.dedup();

                let mut changed = false;
                for plugin_id in plugin_ids_to_install {
                    ZedPluginStore::install_registry_plugin(
                        layout.clone(),
                        client.clone(),
                        plugin_id,
                    )
                    .await?;
                    changed = true;
                }

                Ok(changed)
            })
            .await;

        match result {
            Ok(true) => {
                cx.update(|cx| plugin_host::refresh_catalog(cx));
            }
            Ok(false) => {}
            Err(error) => {
                log::error!("failed to sync plugins with registry: {error:#}");
            }
        }

        anyhow::Ok(())
    })
    .detach();
}

#[cfg(test)]
mod tests {
    use super::*;
    use plugin::{InstalledPlugin, PluginInstallSource, PluginInstallation, PluginManifest};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_layout() -> plugin::PluginStoreLayout {
        plugin::PluginStoreLayout {
            installed_root: PathBuf::from("/tmp/plugins/installed"),
            development_root: PathBuf::from("/tmp/plugins/development"),
        }
    }

    fn test_plugin(root: PathBuf, state: PluginState) -> InstalledPlugin {
        let manifest = test_manifest();

        InstalledPlugin {
            manifest,
            state,
            installation: PluginInstallation {
                root: root.clone(),
                source: PluginInstallSource::Directory(root),
            },
            error_message: Some("manifest failure".into()),
        }
    }

    fn test_manifest() -> PluginManifest {
        let manifest_root = std::env::temp_dir().join(format!(
            "neo-zed-plugin-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(manifest_root.join("bin")).unwrap();
        fs::write(
            manifest_root.join("plugin.toml"),
            r#"
id = "acme-test-plugin"
name = "Test Plugin"
version = "0.1.0"
schema_version = 1
entry = "bin/test-plugin"
"#,
        )
        .unwrap();
        fs::write(manifest_root.join("bin/test-plugin"), "#!/bin/sh\n").unwrap();

        let manifest = PluginManifest::load(&manifest_root).unwrap();
        fs::remove_dir_all(&manifest_root).unwrap();
        manifest
    }

    #[test]
    fn broken_development_plugin_stays_in_development_source() {
        let layout = test_layout();
        let plugin = test_plugin(
            PathBuf::from("/Users/nest/Developer/neo-zed/plugins/codex-usage-plugin"),
            PluginState::Error,
        );

        assert!(matches!(
            infer_plugin_source(&layout, &plugin),
            PluginSource::Development { .. }
        ));
    }

    #[test]
    fn broken_installed_plugin_stays_in_registry_source() {
        let layout = test_layout();
        let plugin = test_plugin(
            layout.installed_root.join("codex-usage-plugin"),
            PluginState::Error,
        );

        assert!(matches!(
            infer_plugin_source(&layout, &plugin),
            PluginSource::Registry
        ));
    }
}
