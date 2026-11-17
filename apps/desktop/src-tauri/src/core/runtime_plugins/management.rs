//! Local package installation and persistent selection. This process keeps its
//! startup selection so existing Native bindings and admitted work remain valid.
pub(crate) mod commands;
mod install;
mod store;
#[cfg(test)]
mod tests;

use super::{
    PackageSnapshot, RuntimePluginCatalog, RuntimePluginError, RuntimePluginPackage, files, scan,
};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use store::{Registry, Selection};

type Discovered = Vec<(RuntimePluginPackage, PackageSnapshot)>;

pub(super) struct ManagedPlugins {
    root: PathBuf,
    startup: Result<Registry, RuntimePluginError>,
    _leases: Vec<fs::File>,
}
impl ManagedPlugins {
    pub fn open(root: PathBuf) -> Self {
        let mut leases = Vec::new();
        let startup = (|| {
            let guard = store::registry(&root)?;
            for row in guard
                .state
                .plugins
                .values()
                .filter(|row| row.enabled && !row.removed)
            {
                if let Some(key) = &row.package_key {
                    leases.push(store::lease(&root, key)?);
                }
            }
            // Failure to reclaim old bytes must not withdraw valid installations.
            if let Err(error) = store::collect(&root, &guard.state) {
                eprintln!("插件缓存回收 [{}]：{}", error.code, error.message);
            }
            Ok(guard.state.clone())
        })();
        Self {
            root,
            startup,
            _leases: leases,
        }
    }
    pub fn select(&self, mut base: Discovered) -> Result<Discovered, RuntimePluginError> {
        let registry = self.startup.as_ref().map_err(Clone::clone)?;
        base.retain(|(package, _)| {
            package
                .manifest
                .get("id")
                .and_then(serde_json::Value::as_str)
                .and_then(|id| registry.plugins.get(id))
                .is_none_or(|row| !row.removed && row.enabled && row.package_key.is_none())
        });
        for (id, row) in &registry.plugins {
            if row.removed || !row.enabled {
                continue;
            }
            if let Some(key) = &row.package_key {
                base.push(self.read(id, key)?);
            }
        }
        Ok(base)
    }
    fn read(
        &self,
        id: &str,
        key: &str,
    ) -> Result<(RuntimePluginPackage, PackageSnapshot), RuntimePluginError> {
        let root = files::controlled_root(&self.root.join("packages"))?;
        let (package, snapshot) =
            scan::read_package(&root.join(key), &root).map_err(|error| error.package(key))?;
        if package.package_key != key || install::identity(&package)? != id {
            return Err(store::error(
                "runtime_plugin_resource_changed",
                "已安装插件的内容身份发生变化。",
            )
            .package(key));
        }
        Ok((package, snapshot))
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedPluginRow {
    pub package: RuntimePluginPackage,
    pub enabled: bool,
    pub removed: bool,
    pub startup_package_key: Option<String>,
    pub origin: &'static str,
    pub diagnostic: Option<RuntimePluginError>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagedPluginListing {
    pub revision: String,
    pub plugins: Vec<ManagedPluginRow>,
}

impl RuntimePluginCatalog {
    pub(crate) fn attach_managed_root(&self, root: PathBuf) {
        self.management.get_or_init(|| ManagedPlugins::open(root));
    }
    fn manager(&self) -> Result<&ManagedPlugins, RuntimePluginError> {
        self.management.get().ok_or_else(|| {
            store::error(
                "plugin_management_unavailable",
                "本地插件管理目录尚未初始化。",
            )
        })
    }
    pub(super) fn bundled_packages(&self) -> Result<Discovered, RuntimePluginError> {
        match self.root.as_ref().or_else(|| self.bundled_root.get()) {
            Some(root) => scan::discover(root),
            None => Ok(Vec::new()),
        }
    }
    pub(crate) fn managed_plugins(&self) -> Result<ManagedPluginListing, RuntimePluginError> {
        let manager = self.manager()?;
        let guard = store::registry(&manager.root)?;
        let (active, startup_error) = match self.discover() {
            Ok(listing) => (listing.packages, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        let mut by_id = BTreeMap::new();
        let mut failures = BTreeMap::new();
        for (package, _) in self.bundled_packages()? {
            let id = install::identity(&package)?.to_owned();
            if by_id.insert(id, package).is_some() {
                return Err(store::error(
                    "plugin_package_ambiguous",
                    "配置目录中存在重复插件 ID，请先整理构建目录。",
                ));
            }
        }
        for (id, row) in &guard.state.plugins {
            if row.removed {
                by_id.remove(id);
                if let Some(package) = active
                    .iter()
                    .find(|package| package.manifest["id"].as_str() == Some(id))
                {
                    by_id.insert(id.clone(), package.clone());
                }
            } else if let Some(key) = &row.package_key {
                match manager.read(id, key) {
                    Ok((package, _)) => {
                        by_id.insert(id.clone(), package);
                    }
                    Err(error) => {
                        let manifest = row
                            .manifest
                            .clone()
                            .unwrap_or_else(|| serde_json::json!({"id": id, "version": "未知"}));
                        by_id.insert(
                            id.clone(),
                            RuntimePluginPackage {
                                package_key: key.clone(),
                                manifest,
                            },
                        );
                        failures.insert(id.clone(), error);
                    }
                }
            }
        }
        let plugins = by_id
            .into_iter()
            .map(|(id, package)| {
                let row = guard.state.plugins.get(&id);
                ManagedPluginRow {
                    diagnostic: failures.remove(&id).or_else(|| {
                        if row.is_none_or(|row| row.enabled) {
                            startup_error.clone()
                        } else {
                            None
                        }
                    }),
                    startup_package_key: active
                        .iter()
                        .find(|package| package.manifest["id"].as_str() == Some(&id))
                        .map(|package| package.package_key.clone()),
                    enabled: row.is_none_or(|row| row.enabled),
                    removed: row.is_some_and(|row| row.removed),
                    origin: if row.is_some_and(|row| row.package_key.is_some()) {
                        "installed"
                    } else {
                        "bundled"
                    },
                    package,
                }
            })
            .collect();
        Ok(ManagedPluginListing {
            revision: guard.revision,
            plugins,
        })
    }

    pub(crate) fn install_managed_plugin(
        &self,
        source: &Path,
        expected_revision: &str,
    ) -> Result<RuntimePluginPackage, RuntimePluginError> {
        let manager = self.manager()?;
        let staged = install::stage(source, &manager.root)?;
        let id = install::identity(&staged.package)?.to_owned();
        let mut guard = store::registry(&manager.root)?;
        guard.expect(expected_revision)?;
        if !guard.state.plugins.contains_key(&id) && guard.state.plugins.len() >= 128 {
            return Err(store::error(
                "runtime_plugin_catalog_limit",
                "本地安装记录已达到128项限制。",
            ));
        }
        let packages = store::directory(&manager.root.join("packages"))?;
        let target = packages.join(&staged.package.package_key);
        if target.exists() {
            manager.read(&id, &staged.package.package_key)?;
        } else {
            fs::rename(&staged.snapshot.directory, &target)
                .map_err(|e| files::io("插件包提交", e))?;
            #[cfg(unix)]
            fs::File::open(&packages)
                .and_then(|file| file.sync_all())
                .map_err(|e| files::io("插件包目录落盘", e))?;
        }
        let enabled = match guard.state.plugins.get(&id) {
            Some(row) => {
                !row.removed
                    && row.enabled
                    && row.package_key.as_deref() == Some(staged.package.package_key.as_str())
            }
            None => self
                .bundled_packages()?
                .iter()
                .any(|(package, _)| package.package_key == staged.package.package_key),
        };
        guard.state.plugins.insert(
            id,
            Selection {
                package_key: Some(staged.package.package_key.clone()),
                enabled,
                removed: false,
                manifest: Some(staged.package.manifest.clone()),
            },
        );
        guard.save()?;
        Ok(staged.package)
    }

    pub(crate) fn set_managed_plugin(
        &self,
        id: &str,
        enabled: bool,
        removed: bool,
        expected_revision: &str,
    ) -> Result<(), RuntimePluginError> {
        if !store::valid_id(id) {
            return Err(store::error("plugin_identity_invalid", "插件身份无效。"));
        }
        let manager = self.manager()?;
        let mut guard = store::registry(&manager.root)?;
        guard.expect(expected_revision)?;
        let previous = guard.state.plugins.get(id).cloned();
        let key = match &previous {
            Some(row) if !row.removed => row.package_key.clone(),
            Some(_) => {
                return Err(store::error(
                    "plugin_not_installed",
                    "插件已安排卸载，请刷新列表。",
                ));
            }
            None => {
                if !self
                    .bundled_packages()?
                    .iter()
                    .any(|(package, _)| package.manifest["id"].as_str() == Some(id))
                {
                    return Err(store::error("plugin_not_installed", "插件尚未安装。"));
                }
                None
            }
        };
        guard.state.plugins.insert(
            id.to_owned(),
            Selection {
                package_key: key,
                enabled: enabled && !removed,
                removed,
                manifest: previous.and_then(|row| row.manifest),
            },
        );
        guard.save()
    }
}
