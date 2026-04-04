use std::fs;
use std::path::PathBuf;

use plugin::{PluginInstallSource, PluginManifest, PluginState, PluginStore, PluginStoreLayout};
use pretty_assertions::assert_eq;
use tempfile::TempDir;

#[test]
fn discovers_plugin_manifest_from_local_directory() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin("source-plugin", "0.1.0");

    let manifest = PluginManifest::load(fixture.plugin_dir("source-plugin")).unwrap();

    assert_eq!(manifest.id.as_str(), "acme-test-panel");
    assert_eq!(manifest.name, "Test Panel");
    assert_eq!(manifest.version, "0.1.0");
    assert_eq!(manifest.authors, vec!["Neo Zed"]);
    assert_eq!(
        manifest.repository.as_deref(),
        Some("https://github.com/Nkr1shna/neo-zed")
    );
    assert_eq!(
        manifest.homepage.as_deref(),
        Some("https://example.com/plugins/acme-test-panel")
    );
    assert_eq!(manifest.entrypoint, PathBuf::from("bin/test-panel"));
    assert_eq!(manifest.titlebar_widgets.len(), 1);
    assert_eq!(manifest.titlebar_widgets[0].id, "search-widget");
    assert_eq!(manifest.display_name(), "Test Panel");
}

#[test]
fn install_and_remove_update_bookkeeping() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin("source-plugin", "0.1.0");
    let mut store = fixture.store();

    let installed = store
        .install_from_directory(fixture.plugin_dir("source-plugin"))
        .unwrap();

    assert_eq!(installed.state, PluginState::Installed);
    assert_eq!(
        installed.installation.source,
        PluginInstallSource::Directory(fixture.plugin_dir("source-plugin"))
    );
    assert!(
        fixture
            .installed_plugin_dir("acme-test-panel")
            .join("plugin.toml")
            .exists()
    );

    let removed = store.remove("acme-test-panel").unwrap();

    assert!(removed);
    assert!(!fixture.installed_plugin_dir("acme-test-panel").exists());
}

#[test]
fn enumerates_installed_and_dev_plugins_with_expected_states() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin("installed-source", "0.1.0");
    fixture.write_plugin("dev-source", "0.2.0-dev");

    let mut store = fixture.store();
    store
        .install_from_directory(fixture.plugin_dir("installed-source"))
        .unwrap();
    store
        .register_development_plugin(fixture.plugin_dir("dev-source"))
        .unwrap();

    let plugins = store.list().unwrap();

    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[0].manifest.version, "0.2.0-dev");
    assert_eq!(plugins[0].state, PluginState::Development);
    assert_eq!(plugins[0].manifest.titlebar_widgets.len(), 1);
    assert_eq!(plugins[1].manifest.version, "0.1.0");
    assert_eq!(plugins[1].state, PluginState::Installed);
}

#[test]
fn reinstalling_plugin_updates_status_and_replaces_existing_copy() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin("source-plugin", "0.1.0");

    let mut store = fixture.store();
    let first = store
        .install_from_directory(fixture.plugin_dir("source-plugin"))
        .unwrap();
    assert_eq!(first.state, PluginState::Installed);

    fixture.write_plugin("source-plugin", "0.2.0");

    let second = store
        .install_from_directory(fixture.plugin_dir("source-plugin"))
        .unwrap();
    let plugins = store.list().unwrap();

    assert_eq!(second.manifest.version, "0.2.0");
    assert_eq!(plugins.len(), 1);
    assert_eq!(plugins[0].manifest.version, "0.2.0");
}

#[test]
fn manifest_rejects_unsupported_schema_version() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin_manifest("source-plugin", "0.1.0", Some(2), Some("bin/test-panel"));
    fixture.write_entrypoint("source-plugin");

    let error = PluginManifest::load(fixture.plugin_dir("source-plugin")).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("uses unsupported schema_version 2")
    );
}

#[test]
fn install_requires_manifest_entrypoint_to_exist() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin_manifest("source-plugin", "0.1.0", Some(1), Some("bin/test-panel"));
    let mut store = fixture.store();

    let error = store
        .install_from_directory(fixture.plugin_dir("source-plugin"))
        .unwrap_err();

    assert!(error.to_string().contains("references missing entrypoint"));
}

#[test]
fn manifest_rejects_invalid_plugin_ids() {
    for invalid_plugin_id in ["../escape", "foo/bar", "foo.bar"] {
        let fixture = PluginFixture::new(invalid_plugin_id);
        fixture.write_plugin("source-plugin", "0.1.0");

        let error = PluginManifest::load(fixture.plugin_dir("source-plugin")).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("must use an id containing only ASCII alphanumeric characters")
        );
    }
}

#[test]
fn install_rejects_entrypoint_path_traversal() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin_manifest("source-plugin", "0.1.0", Some(1), Some("../escape.sh"));
    fs::write(fixture.root.path().join("escape.sh"), "#!/bin/sh\n").unwrap();

    let mut store = fixture.store();
    let error = store
        .install_from_directory(fixture.plugin_dir("source-plugin"))
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("must not contain path traversal")
    );
}

#[cfg(unix)]
#[test]
fn install_rejects_entrypoint_symlink_escape() {
    use std::os::unix::fs::symlink;

    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin_manifest("source-plugin", "0.1.0", Some(1), Some("bin/test-panel"));
    fs::create_dir_all(fixture.plugin_dir("source-plugin").join("bin")).unwrap();
    fs::write(fixture.root.path().join("escape.sh"), "#!/bin/sh\n").unwrap();
    symlink(
        "../../escape.sh",
        fixture.plugin_dir("source-plugin").join("bin/test-panel"),
    )
    .unwrap();

    let mut store = fixture.store();
    let error = store
        .install_from_directory(fixture.plugin_dir("source-plugin"))
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("resolves outside plugin directory")
    );
}

#[cfg(unix)]
#[test]
fn install_rejects_symlinked_source_directory() {
    use std::os::unix::fs::symlink;

    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin("source-plugin", "0.1.0");
    let symlinked_source = fixture.root.path().join("symlinked-source");
    symlink(fixture.plugin_dir("source-plugin"), &symlinked_source).unwrap();

    let mut store = fixture.store();
    let error = store.install_from_directory(&symlinked_source).unwrap_err();

    assert!(error.to_string().contains("must not be a symlink"));
}

#[cfg(unix)]
#[test]
fn install_rejects_symlink_inside_plugin_directory() {
    use std::os::unix::fs::symlink;

    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin("source-plugin", "0.1.0");
    fs::write(fixture.root.path().join("escape.sh"), "#!/bin/sh\n").unwrap();
    symlink(
        "../../escape.sh",
        fixture.plugin_dir("source-plugin").join("bin/escape"),
    )
    .unwrap();

    let mut store = fixture.store();
    let error = store
        .install_from_directory(fixture.plugin_dir("source-plugin"))
        .unwrap_err();

    assert!(error.to_string().contains("contains symlink"));
}

#[cfg(unix)]
#[test]
fn install_rejects_hard_link_inside_plugin_directory() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin("source-plugin", "0.1.0");
    fs::write(fixture.root.path().join("escape.sh"), "#!/bin/sh\n").unwrap();
    fs::hard_link(
        fixture.root.path().join("escape.sh"),
        fixture.plugin_dir("source-plugin").join("bin/escape"),
    )
    .unwrap();

    let mut store = fixture.store();
    let error = store
        .install_from_directory(fixture.plugin_dir("source-plugin"))
        .unwrap_err();

    assert!(error.to_string().contains("hard linked file"));
}

#[test]
fn cargo_plugin_manifest_accepts_declared_bin_target() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_cargo_plugin("source-plugin", "0.1.0", "test-panel-plugin");

    let manifest = PluginManifest::load(fixture.plugin_dir("source-plugin")).unwrap();

    assert_eq!(manifest.entrypoint, PathBuf::from("test-panel-plugin"));
}

#[test]
fn cargo_plugin_manifest_requires_declared_bin_target() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_cargo_plugin_manifest("source-plugin", "0.1.0", "missing-bin");

    let error = PluginManifest::load(fixture.plugin_dir("source-plugin")).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("references missing Cargo bin target `missing-bin`")
    );
}

#[test]
fn list_surfaces_installed_manifest_errors_without_failing() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin("healthy-plugin", "0.1.0");

    let mut store = fixture.store();
    store
        .install_from_directory(fixture.plugin_dir("healthy-plugin"))
        .unwrap();

    let broken_directory = fixture.installed_plugin_dir("broken-plugin");
    fs::create_dir_all(&broken_directory).unwrap();
    fs::write(broken_directory.join("plugin.toml"), "this is not toml").unwrap();

    let plugins = store.list().unwrap();

    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[0].state, PluginState::Installed);
    assert_eq!(plugins[1].state, PluginState::Error);
    assert_eq!(plugins[1].manifest.id.as_str(), "broken-plugin");
    assert!(
        plugins[1]
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("failed to parse plugin manifest")
    );
}

#[test]
fn list_surfaces_development_registration_errors_without_failing() {
    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin("healthy-plugin", "0.1.0");

    let mut store = fixture.store();
    store
        .register_development_plugin(fixture.plugin_dir("healthy-plugin"))
        .unwrap();

    let broken_registration = fixture
        .root
        .path()
        .join("development")
        .join("broken-plugin.json");
    fs::create_dir_all(broken_registration.parent().unwrap()).unwrap();
    fs::write(&broken_registration, "{ not json").unwrap();

    let plugins = store.list().unwrap();

    assert_eq!(plugins.len(), 2);
    assert_eq!(plugins[0].state, PluginState::Development);
    assert_eq!(plugins[1].state, PluginState::Error);
    assert_eq!(plugins[1].manifest.id.as_str(), "broken-plugin");
    assert!(
        plugins[1]
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("failed to parse development registration")
    );
}

#[cfg(unix)]
#[test]
fn failed_reinstall_keeps_existing_installed_copy() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = PluginFixture::new("acme-test-panel");
    fixture.write_plugin("installed-plugin", "0.1.0");
    fixture.write_plugin("replacement-plugin", "0.2.0");
    fixture.write_unreadable_asset("replacement-plugin");

    let mut store = fixture.store();
    store
        .install_from_directory(fixture.plugin_dir("installed-plugin"))
        .unwrap();

    fs::set_permissions(
        fixture
            .plugin_dir("replacement-plugin")
            .join("assets/unreadable.txt"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();

    let error = store
        .install_from_directory(fixture.plugin_dir("replacement-plugin"))
        .unwrap_err();
    let manifest = PluginManifest::load(fixture.installed_plugin_dir("acme-test-panel")).unwrap();

    assert!(error.to_string().contains("failed to copy plugin file"));
    assert_eq!(manifest.version, "0.1.0");

    fs::set_permissions(
        fixture
            .plugin_dir("replacement-plugin")
            .join("assets/unreadable.txt"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
}

struct PluginFixture {
    root: TempDir,
    plugin_id: String,
}

impl PluginFixture {
    fn new(plugin_id: &str) -> Self {
        Self {
            root: TempDir::new().unwrap(),
            plugin_id: plugin_id.into(),
        }
    }

    fn store(&self) -> PluginStore {
        PluginStore::new(PluginStoreLayout {
            installed_root: self.root.path().join("installed"),
            development_root: self.root.path().join("development"),
        })
    }

    fn write_plugin(&self, directory_name: &str, version: &str) {
        self.write_plugin_manifest(directory_name, version, Some(1), Some("bin/test-panel"));
        self.write_entrypoint(directory_name);
    }

    fn write_plugin_manifest(
        &self,
        directory_name: &str,
        version: &str,
        schema_version: Option<u32>,
        entrypoint: Option<&str>,
    ) {
        let plugin_dir = self.plugin_dir(directory_name);
        fs::create_dir_all(&plugin_dir).unwrap();
        let schema_version = schema_version.unwrap_or(1);
        let entrypoint = entrypoint.unwrap_or("bin/test-panel");
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                r#"
id = "{plugin_id}"
name = "Test Panel"
version = "{version}"
schema_version = {schema_version}
authors = ["Neo Zed"]
repository = "https://github.com/Nkr1shna/neo-zed"
homepage = "https://example.com/plugins/{plugin_id}"
entrypoint = "{entrypoint}"
description = "Test plugin"

[[panels]]
id = "deploy-panel"
title = "Deploy"
dock = "right"
activation = "on_demand"

[[titlebar_widgets]]
id = "search-widget"
title = "Search"
icon_name = "search"
tooltip = "Open search widget"
side = "right"
priority = 10
opens_panel_id = "deploy-panel"
"#,
                plugin_id = self.plugin_id,
                version = version,
                schema_version = schema_version,
                entrypoint = entrypoint,
            ),
        )
        .unwrap();
    }

    fn write_entrypoint(&self, directory_name: &str) {
        let plugin_dir = self.plugin_dir(directory_name);
        fs::create_dir_all(plugin_dir.join("bin")).unwrap();
        fs::write(plugin_dir.join("bin/test-panel"), "#!/bin/sh\n").unwrap();
    }

    fn write_cargo_plugin(&self, directory_name: &str, version: &str, bin_name: &str) {
        self.write_cargo_plugin_manifest(directory_name, version, bin_name);
        self.write_cargo_entrypoint(directory_name);
    }

    fn write_cargo_plugin_manifest(&self, directory_name: &str, version: &str, bin_name: &str) {
        let plugin_dir = self.plugin_dir(directory_name);
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                r#"
id = "{plugin_id}"
name = "Test Panel"
version = "{version}"
schema_version = 1
entry = "{bin_name}"
description = "Cargo plugin"
"#,
                plugin_id = self.plugin_id,
                version = version,
                bin_name = bin_name,
            ),
        )
        .unwrap();
        fs::write(
            plugin_dir.join("Cargo.toml"),
            r#"
[package]
name = "test-panel-plugin"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "test-panel-plugin"
path = "src/main.rs"
"#,
        )
        .unwrap();
    }

    fn write_cargo_entrypoint(&self, directory_name: &str) {
        let plugin_dir = self.plugin_dir(directory_name);
        fs::create_dir_all(plugin_dir.join("src")).unwrap();
        fs::write(plugin_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    }

    #[cfg(unix)]
    fn write_unreadable_asset(&self, directory_name: &str) {
        let asset_directory = self.plugin_dir(directory_name).join("assets");
        fs::create_dir_all(&asset_directory).unwrap();
        fs::write(asset_directory.join("unreadable.txt"), "secret").unwrap();
    }

    fn plugin_dir(&self, directory_name: &str) -> PathBuf {
        self.root.path().join(directory_name)
    }
    fn installed_plugin_dir(&self, plugin_id: &str) -> PathBuf {
        self.root.path().join("installed").join(plugin_id)
    }
}
