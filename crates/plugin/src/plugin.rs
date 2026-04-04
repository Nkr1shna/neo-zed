use std::collections::BTreeSet;
use std::fmt;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use plugin_protocol::{PanelDescriptor, PluginId, PluginInstallState, TitlebarWidgetDescriptor};
use serde::{Deserialize, Serialize};

pub use plugin_protocol::PluginInstallState as PluginState;

const PLUGIN_MANIFEST_NAME: &str = "plugin.toml";
const INSTALLATION_METADATA_FILE: &str = ".plugin-installation.json";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginStoreLayout {
    pub installed_root: PathBuf,
    pub development_root: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginInstallSource {
    Directory(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInstallation {
    pub root: PathBuf,
    pub source: PluginInstallSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledPlugin {
    pub manifest: PluginManifest,
    pub state: PluginState,
    pub installation: PluginInstallation,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug)]
pub struct PluginStore {
    layout: PluginStoreLayout,
}

impl PluginStore {
    pub fn new(layout: PluginStoreLayout) -> Self {
        Self { layout }
    }

    pub fn install_from_directory(
        &mut self,
        source_directory: impl AsRef<Path>,
    ) -> Result<InstalledPlugin> {
        let source_directory = source_directory.as_ref();
        ensure_not_symlink(source_directory)?;
        let manifest = PluginManifest::load(source_directory)?;
        let installed_directory = self.layout.installed_root.join(manifest.id.as_str());
        let staging_directory = unique_temporary_directory(&installed_directory, "installing");

        ensure_directory(&self.layout.installed_root)?;

        if let Err(error) = copy_directory(source_directory, &staging_directory).and_then(|_| {
            PluginManifest::load(&staging_directory)?;
            Ok(())
        }) {
            remove_directory_if_exists(&staging_directory).with_context(|| {
                format!(
                    "failed to clean up staged plugin installation {}",
                    staging_directory.display()
                )
            })?;
            return Err(error);
        }

        let installation = PluginInstallation {
            root: installed_directory.clone(),
            source: PluginInstallSource::Directory(source_directory.to_path_buf()),
        };
        if let Err(error) = write_installation_metadata(&staging_directory, &installation) {
            remove_directory_if_exists(&staging_directory).with_context(|| {
                format!(
                    "failed to clean up staged plugin installation {}",
                    staging_directory.display()
                )
            })?;
            return Err(error);
        }
        replace_installed_directory(&staging_directory, &installed_directory)?;

        Ok(InstalledPlugin {
            manifest,
            state: PluginInstallState::Installed,
            installation,
            error_message: None,
        })
    }

    pub fn register_development_plugin(
        &mut self,
        source_directory: impl AsRef<Path>,
    ) -> Result<InstalledPlugin> {
        let source_directory = source_directory.as_ref();
        ensure_not_symlink(source_directory)?;
        let manifest = PluginManifest::load(source_directory)?;
        let registration_path = self.development_registration_path(manifest.id.as_str());
        let installation = PluginInstallation {
            root: source_directory.to_path_buf(),
            source: PluginInstallSource::Directory(source_directory.to_path_buf()),
        };

        ensure_directory(&self.layout.development_root)?;
        fs::write(
            &registration_path,
            serde_json::to_vec_pretty(&installation)
                .context("failed to serialize development plugin registration")?,
        )
        .with_context(|| {
            format!(
                "failed to write development plugin registration {}",
                registration_path.display()
            )
        })?;

        Ok(InstalledPlugin {
            manifest,
            state: PluginInstallState::Development,
            installation,
            error_message: None,
        })
    }

    pub fn remove(&mut self, plugin_id: &str) -> Result<bool> {
        let mut removed_anything = false;
        let installed_directory = self.layout.installed_root.join(plugin_id);
        if installed_directory.exists() {
            fs::remove_dir_all(&installed_directory).with_context(|| {
                format!(
                    "failed to remove installed plugin {}",
                    installed_directory.display()
                )
            })?;
            removed_anything = true;
        }

        let registration_path = self.development_registration_path(plugin_id);
        if registration_path.exists() {
            fs::remove_file(&registration_path).with_context(|| {
                format!(
                    "failed to remove development registration {}",
                    registration_path.display()
                )
            })?;
            removed_anything = true;
        }

        Ok(removed_anything)
    }

    pub fn list(&self) -> Result<Vec<InstalledPlugin>> {
        let mut plugins = Vec::new();
        plugins.extend(self.list_registered_development_plugins()?);
        plugins.extend(self.list_installed_plugins()?);
        plugins.sort_by(|left, right| {
            installation_state_rank(left.state)
                .cmp(&installation_state_rank(right.state))
                .then_with(|| left.manifest.name.cmp(&right.manifest.name))
                .then_with(|| left.manifest.version.cmp(&right.manifest.version).reverse())
        });
        Ok(plugins)
    }

    fn list_installed_plugins(&self) -> Result<Vec<InstalledPlugin>> {
        if !self.layout.installed_root.exists() {
            return Ok(Vec::new());
        }

        let mut plugins = Vec::new();
        for directory_entry in fs::read_dir(&self.layout.installed_root).with_context(|| {
            format!(
                "failed to read installed plugins directory {}",
                self.layout.installed_root.display()
            )
        })? {
            let directory_entry =
                directory_entry.context("failed to read installed plugin entry")?;
            if !directory_entry
                .file_type()
                .context("failed to inspect installed plugin entry type")?
                .is_dir()
            {
                continue;
            }

            let plugin_root = directory_entry.path();
            match load_installed_plugin(&plugin_root) {
                Ok(plugin) => plugins.push(plugin),
                Err(error) => plugins.push(installed_plugin_error(
                    plugin_directory_name(&plugin_root),
                    PluginInstallation {
                        root: plugin_root.clone(),
                        source: PluginInstallSource::Directory(plugin_root.clone()),
                    },
                    error,
                )),
            }
        }

        Ok(plugins)
    }

    fn list_registered_development_plugins(&self) -> Result<Vec<InstalledPlugin>> {
        if !self.layout.development_root.exists() {
            return Ok(Vec::new());
        }

        let mut plugins = Vec::new();
        for directory_entry in fs::read_dir(&self.layout.development_root).with_context(|| {
            format!(
                "failed to read development plugins directory {}",
                self.layout.development_root.display()
            )
        })? {
            let directory_entry =
                directory_entry.context("failed to read development plugin registration")?;
            if !directory_entry
                .file_type()
                .context("failed to inspect development plugin registration type")?
                .is_file()
            {
                continue;
            }

            let registration_path = directory_entry.path();
            match load_registered_development_plugin(&registration_path) {
                Ok(plugin) => plugins.push(plugin),
                Err(error) => plugins.push(installed_plugin_error(
                    registration_plugin_id(&registration_path),
                    load_development_registration(&registration_path).unwrap_or(
                        PluginInstallation {
                            root: registration_path.clone(),
                            source: PluginInstallSource::Directory(registration_path.clone()),
                        },
                    ),
                    error,
                )),
            }
        }

        Ok(plugins)
    }

    fn development_registration_path(&self, plugin_id: &str) -> PathBuf {
        self.layout
            .development_root
            .join(format!("{plugin_id}.json"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: PluginId,
    pub name: String,
    pub version: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub repository: Option<String>,
    #[serde(default)]
    pub homepage: Option<String>,
    #[serde(alias = "entry")]
    pub entrypoint: PathBuf,
    #[serde(default)]
    pub panels: Vec<PanelDescriptor>,
    #[serde(default)]
    pub titlebar_widgets: Vec<TitlebarWidgetDescriptor>,
}

impl PluginManifest {
    pub fn load(plugin_directory: impl AsRef<Path>) -> Result<Self> {
        let plugin_directory = plugin_directory.as_ref();
        let manifest_path = plugin_directory.join(PLUGIN_MANIFEST_NAME);
        let manifest_contents = fs::read_to_string(&manifest_path).with_context(|| {
            format!("failed to read plugin manifest {}", manifest_path.display())
        })?;
        toml::from_str::<Self>(&manifest_contents)
            .with_context(|| {
                format!(
                    "failed to parse plugin manifest {}",
                    manifest_path.display()
                )
            })?
            .validate(plugin_directory)
    }

    pub fn display_name(&self) -> &str {
        &self.name
    }

    fn validate(self, plugin_directory: &Path) -> Result<Self> {
        validate_plugin_id(plugin_directory, self.id.as_str())?;

        if self.name.trim().is_empty() {
            bail!(
                "plugin manifest {} must define a non-empty name",
                plugin_directory.join(PLUGIN_MANIFEST_NAME).display()
            );
        }

        if self.version.trim().is_empty() {
            bail!(
                "plugin manifest {} must define a non-empty version",
                plugin_directory.join(PLUGIN_MANIFEST_NAME).display()
            );
        }

        let supported_schema_version = default_schema_version();
        if self.schema_version != supported_schema_version {
            bail!(
                "plugin manifest {} uses unsupported schema_version {} (expected {})",
                plugin_directory.join(PLUGIN_MANIFEST_NAME).display(),
                self.schema_version,
                supported_schema_version
            );
        }

        if self.entrypoint.as_os_str().is_empty() {
            bail!(
                "plugin manifest {} must define a non-empty entrypoint",
                plugin_directory.join(PLUGIN_MANIFEST_NAME).display()
            );
        }

        if self.entrypoint.is_absolute() {
            bail!(
                "plugin manifest {} must use a relative entrypoint path",
                plugin_directory.join(PLUGIN_MANIFEST_NAME).display()
            );
        }

        validate_plugin_entrypoint(plugin_directory, &self.entrypoint)?;

        Ok(self)
    }
}

impl fmt::Display for PluginManifest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} {}", self.name, self.version)
    }
}

fn default_schema_version() -> u32 {
    1
}

#[derive(Debug, Deserialize)]
struct CargoManifestMetadata {
    package: Option<CargoPackageMetadata>,
    #[serde(default, rename = "bin")]
    bins: Vec<CargoBinMetadata>,
}

#[derive(Debug, Deserialize)]
struct CargoPackageMetadata {
    name: Option<String>,
    autobins: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CargoBinMetadata {
    name: Option<String>,
}

fn validate_plugin_entrypoint(plugin_directory: &Path, entrypoint: &Path) -> Result<()> {
    let cargo_manifest_path = plugin_directory.join("Cargo.toml");
    if cargo_manifest_path.exists() {
        validate_cargo_entrypoint(plugin_directory, &cargo_manifest_path, entrypoint)
    } else {
        validate_executable_entrypoint(plugin_directory, entrypoint)
    }
}

fn validate_executable_entrypoint(plugin_directory: &Path, entrypoint: &Path) -> Result<()> {
    validate_relative_entrypoint_path(plugin_directory, entrypoint)?;

    let entrypoint_path = plugin_directory.join(entrypoint);
    let entrypoint_metadata = fs::metadata(&entrypoint_path).with_context(|| {
        format!(
            "plugin manifest {} references missing entrypoint {}",
            plugin_directory.join(PLUGIN_MANIFEST_NAME).display(),
            entrypoint_path.display()
        )
    })?;
    if !entrypoint_metadata.is_file() {
        bail!(
            "plugin manifest {} entrypoint {} is not a file",
            plugin_directory.join(PLUGIN_MANIFEST_NAME).display(),
            entrypoint_path.display()
        );
    }

    let canonical_plugin_directory = fs::canonicalize(plugin_directory).with_context(|| {
        format!(
            "failed to resolve plugin directory {}",
            plugin_directory.display()
        )
    })?;
    let canonical_entrypoint_path = fs::canonicalize(&entrypoint_path).with_context(|| {
        format!(
            "plugin manifest {} references entrypoint {} that cannot be resolved",
            plugin_directory.join(PLUGIN_MANIFEST_NAME).display(),
            entrypoint_path.display()
        )
    })?;
    if !canonical_entrypoint_path.starts_with(&canonical_plugin_directory) {
        bail!(
            "plugin manifest {} entrypoint {} resolves outside plugin directory {}",
            plugin_directory.join(PLUGIN_MANIFEST_NAME).display(),
            entrypoint_path.display(),
            plugin_directory.display()
        );
    }

    Ok(())
}

fn validate_relative_entrypoint_path(plugin_directory: &Path, entrypoint: &Path) -> Result<()> {
    if entrypoint.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        bail!(
            "plugin manifest {} must not contain path traversal in `entrypoint`",
            plugin_directory.join(PLUGIN_MANIFEST_NAME).display()
        );
    }

    Ok(())
}

fn validate_plugin_id(plugin_directory: &Path, plugin_id: &str) -> Result<()> {
    if plugin_id.trim().is_empty() {
        bail!(
            "plugin manifest {} must define a non-empty id",
            plugin_directory.join(PLUGIN_MANIFEST_NAME).display()
        );
    }

    if !plugin_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        bail!(
            "plugin manifest {} must use an id containing only ASCII alphanumeric characters, `-`, or `_`",
            plugin_directory.join(PLUGIN_MANIFEST_NAME).display()
        );
    }

    Ok(())
}

fn validate_cargo_entrypoint(
    plugin_directory: &Path,
    cargo_manifest_path: &Path,
    entrypoint: &Path,
) -> Result<()> {
    let entrypoint_name = cargo_entrypoint_name(plugin_directory, entrypoint)?;
    let available_bin_names = discover_cargo_bin_names(plugin_directory, cargo_manifest_path)?;

    if available_bin_names.contains(entrypoint_name) {
        return Ok(());
    }

    let available_bin_names = if available_bin_names.is_empty() {
        "none discovered".to_string()
    } else {
        available_bin_names
            .into_iter()
            .collect::<Vec<_>>()
            .join(", ")
    };

    bail!(
        "plugin manifest {} references missing Cargo bin target `{}` in {} (available: {})",
        plugin_directory.join(PLUGIN_MANIFEST_NAME).display(),
        entrypoint_name,
        cargo_manifest_path.display(),
        available_bin_names
    );
}

fn cargo_entrypoint_name<'a>(plugin_directory: &Path, entrypoint: &'a Path) -> Result<&'a str> {
    let mut components = entrypoint.components();
    let Some(Component::Normal(component)) = components.next() else {
        bail!(
            "plugin manifest {} must use a Cargo bin target name for `entry`, not a path",
            plugin_directory.join(PLUGIN_MANIFEST_NAME).display()
        );
    };
    if components.next().is_some() {
        bail!(
            "plugin manifest {} must use a Cargo bin target name for `entry`, not a path",
            plugin_directory.join(PLUGIN_MANIFEST_NAME).display()
        );
    }

    component.to_str().with_context(|| {
        format!(
            "plugin manifest {} must use a valid UTF-8 Cargo bin target name",
            plugin_directory.join(PLUGIN_MANIFEST_NAME).display()
        )
    })
}

fn discover_cargo_bin_names(
    plugin_directory: &Path,
    cargo_manifest_path: &Path,
) -> Result<BTreeSet<String>> {
    let cargo_manifest_contents = fs::read_to_string(cargo_manifest_path).with_context(|| {
        format!(
            "failed to read Cargo manifest {}",
            cargo_manifest_path.display()
        )
    })?;
    let cargo_manifest = toml::from_str::<CargoManifestMetadata>(&cargo_manifest_contents)
        .with_context(|| {
            format!(
                "failed to parse Cargo manifest {}",
                cargo_manifest_path.display()
            )
        })?;

    let mut bin_names = BTreeSet::new();
    for bin in cargo_manifest.bins {
        if let Some(name) = bin.name.filter(|name| !name.trim().is_empty()) {
            bin_names.insert(name);
        }
    }

    if cargo_manifest
        .package
        .as_ref()
        .and_then(|package| package.autobins)
        .unwrap_or(true)
    {
        if let Some(package_name) = cargo_manifest
            .package
            .as_ref()
            .and_then(|package| package.name.as_deref())
            .filter(|name| !name.trim().is_empty())
        {
            if plugin_directory.join("src/main.rs").is_file() {
                bin_names.insert(package_name.to_string());
            }
        }

        let src_bin_directory = plugin_directory.join("src/bin");
        if src_bin_directory.is_dir() {
            for directory_entry in fs::read_dir(&src_bin_directory).with_context(|| {
                format!(
                    "failed to read Cargo bin directory {}",
                    src_bin_directory.display()
                )
            })? {
                let directory_entry =
                    directory_entry.context("failed to read Cargo bin directory entry")?;
                let path = directory_entry.path();
                let file_type = directory_entry
                    .file_type()
                    .context("failed to inspect Cargo bin entry type")?;

                if file_type.is_file()
                    && path.extension().and_then(|extension| extension.to_str()) == Some("rs")
                {
                    if let Some(bin_name) = path
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .filter(|stem| !stem.is_empty())
                    {
                        bin_names.insert(bin_name.to_string());
                    }
                }

                if file_type.is_dir() && path.join("main.rs").is_file() {
                    if let Some(bin_name) = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .filter(|name| !name.is_empty())
                    {
                        bin_names.insert(bin_name.to_string());
                    }
                }
            }
        }
    }

    Ok(bin_names)
}

fn ensure_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path)
        .with_context(|| format!("failed to create directory {}", path.display()))
}

fn ensure_not_symlink(path: &Path) -> Result<()> {
    if fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect path {}", path.display()))?
        .file_type()
        .is_symlink()
    {
        bail!(
            "plugin source directory {} must not be a symlink",
            path.display()
        );
    }

    Ok(())
}

fn copy_directory(source_directory: &Path, destination_directory: &Path) -> Result<()> {
    ensure_directory(destination_directory)?;
    for directory_entry in fs::read_dir(source_directory)
        .with_context(|| format!("failed to read directory {}", source_directory.display()))?
    {
        let directory_entry = directory_entry.context("failed to read directory entry")?;
        let source_path = directory_entry.path();
        let destination_path = destination_directory.join(directory_entry.file_name());
        let file_type = directory_entry
            .file_type()
            .context("failed to inspect directory entry type")?;

        if file_type.is_symlink() {
            bail!(
                "plugin directory {} contains symlink {}",
                source_directory.display(),
                source_path.display()
            );
        }

        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else {
            ensure_not_hard_link(
                &source_path,
                &directory_entry.metadata().with_context(|| {
                    format!("failed to inspect metadata for {}", source_path.display())
                })?,
            )?;
            if let Some(parent_directory) = destination_path.parent() {
                ensure_directory(parent_directory)?;
            }
            fs::copy(&source_path, &destination_path).with_context(|| {
                format!(
                    "failed to copy plugin file {} to {}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }

    Ok(())
}

fn ensure_not_hard_link(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    if hard_link_count(metadata) > 1 {
        bail!(
            "plugin directory contains hard linked file {}",
            path.display()
        );
    }

    Ok(())
}

#[cfg(unix)]
fn hard_link_count(metadata: &fs::Metadata) -> u64 {
    metadata.nlink()
}

#[cfg(windows)]
fn hard_link_count(metadata: &fs::Metadata) -> u64 {
    metadata.number_of_links().into()
}

#[cfg(not(any(unix, windows)))]
fn hard_link_count(_metadata: &fs::Metadata) -> u64 {
    1
}

fn remove_directory_if_exists(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove directory {}", path.display()))?;
    }
    Ok(())
}

fn replace_installed_directory(staging_directory: &Path, installed_directory: &Path) -> Result<()> {
    let backup_directory = unique_temporary_directory(installed_directory, "backup");
    let had_existing_installation = installed_directory.exists();

    if had_existing_installation {
        fs::rename(installed_directory, &backup_directory).with_context(|| {
            format!(
                "failed to move existing plugin installation {} out of the way",
                installed_directory.display()
            )
        })?;
    }

    if let Err(error) = fs::rename(staging_directory, installed_directory).with_context(|| {
        format!(
            "failed to finalize plugin installation into {}",
            installed_directory.display()
        )
    }) {
        if had_existing_installation {
            fs::rename(&backup_directory, installed_directory).with_context(|| {
                format!(
                    "failed to restore previous plugin installation {} after install error",
                    installed_directory.display()
                )
            })?;
        }

        remove_directory_if_exists(staging_directory).with_context(|| {
            format!(
                "failed to clean up staged plugin installation {}",
                staging_directory.display()
            )
        })?;
        return Err(error);
    }

    if had_existing_installation {
        remove_directory_if_exists(&backup_directory).with_context(|| {
            format!(
                "failed to remove replaced plugin backup {}",
                backup_directory.display()
            )
        })?;
    }

    Ok(())
}

fn unique_temporary_directory(path: &Path, label: &str) -> PathBuf {
    let base_name = path
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .filter(|file_name| !file_name.is_empty())
        .unwrap_or("plugin");
    let parent_directory = path.parent().unwrap_or_else(|| Path::new("."));

    let timestamp_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let process_id = std::process::id();

    for attempt in 0..1000_u32 {
        let candidate = parent_directory.join(format!(
            ".{base_name}.{label}.{process_id}.{timestamp_nanos}.{attempt}"
        ));
        if !candidate.exists() {
            return candidate;
        }
    }

    parent_directory.join(format!(
        ".{base_name}.{label}.{process_id}.{timestamp_nanos}.overflow"
    ))
}

fn write_installation_metadata(
    plugin_root: &Path,
    installation: &PluginInstallation,
) -> Result<()> {
    let metadata_path = plugin_root.join(INSTALLATION_METADATA_FILE);
    fs::write(
        &metadata_path,
        serde_json::to_vec_pretty(installation)
            .context("failed to serialize plugin installation metadata")?,
    )
    .with_context(|| {
        format!(
            "failed to write plugin installation metadata {}",
            metadata_path.display()
        )
    })
}

fn read_installation_metadata(plugin_root: &Path) -> Result<Option<PluginInstallation>> {
    let metadata_path = plugin_root.join(INSTALLATION_METADATA_FILE);
    if !metadata_path.exists() {
        return Ok(None);
    }

    serde_json::from_slice(&fs::read(&metadata_path).with_context(|| {
        format!(
            "failed to read plugin installation metadata {}",
            metadata_path.display()
        )
    })?)
    .map(Some)
    .with_context(|| {
        format!(
            "failed to parse plugin installation metadata {}",
            metadata_path.display()
        )
    })
}

fn load_installed_plugin(plugin_root: &Path) -> Result<InstalledPlugin> {
    let manifest = PluginManifest::load(plugin_root)?;
    let installation = read_installation_metadata(plugin_root)?.unwrap_or(PluginInstallation {
        root: plugin_root.to_path_buf(),
        source: PluginInstallSource::Directory(plugin_root.to_path_buf()),
    });

    Ok(InstalledPlugin {
        manifest,
        state: PluginInstallState::Installed,
        installation,
        error_message: None,
    })
}

fn load_registered_development_plugin(registration_path: &Path) -> Result<InstalledPlugin> {
    let installation = load_development_registration(registration_path)?;
    let manifest = PluginManifest::load(&installation.root)?;
    Ok(InstalledPlugin {
        manifest,
        state: PluginInstallState::Development,
        installation,
        error_message: None,
    })
}

fn load_development_registration(registration_path: &Path) -> Result<PluginInstallation> {
    serde_json::from_slice(&fs::read(registration_path).with_context(|| {
        format!(
            "failed to read development registration {}",
            registration_path.display()
        )
    })?)
    .with_context(|| {
        format!(
            "failed to parse development registration {}",
            registration_path.display()
        )
    })
}

fn installed_plugin_error(
    plugin_id: PluginId,
    installation: PluginInstallation,
    error: anyhow::Error,
) -> InstalledPlugin {
    InstalledPlugin {
        manifest: PluginManifest {
            id: plugin_id,
            name: installation_display_name(&installation),
            version: "unknown".into(),
            schema_version: default_schema_version(),
            description: None,
            authors: Vec::new(),
            repository: None,
            homepage: None,
            entrypoint: PathBuf::new(),
            panels: Vec::new(),
            titlebar_widgets: Vec::new(),
        },
        state: PluginInstallState::Error,
        installation,
        error_message: Some(format!("{error:#}")),
    }
}

fn installation_display_name(installation: &PluginInstallation) -> String {
    installation
        .root
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .filter(|file_name| !file_name.is_empty())
        .unwrap_or("Plugin Load Error")
        .to_string()
}

fn plugin_directory_name(plugin_root: &Path) -> PluginId {
    PluginId::new(
        plugin_root
            .file_name()
            .and_then(|file_name| file_name.to_str())
            .filter(|file_name| !file_name.is_empty())
            .unwrap_or("unknown-plugin"),
    )
}

fn registration_plugin_id(registration_path: &Path) -> PluginId {
    PluginId::new(
        registration_path
            .file_stem()
            .and_then(|file_name| file_name.to_str())
            .filter(|file_name| !file_name.is_empty())
            .unwrap_or("unknown-plugin"),
    )
}

fn installation_state_rank(state: PluginInstallState) -> u8 {
    match state {
        PluginInstallState::Development => 0,
        PluginInstallState::Installed => 1,
        PluginInstallState::Error => 2,
    }
}

pub fn plugin_id_from_directory(plugin_directory: impl AsRef<Path>) -> Result<PluginId> {
    PluginManifest::load(plugin_directory).map(|manifest| manifest.id)
}
