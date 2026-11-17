//! Read-only resources from explicitly configured, content-identified local packages.
//! Manifest compatibility, dependencies and activation remain the Web SDK's job.
pub(crate) mod context_catalog;
pub(crate) mod context_metadata;
pub(crate) mod data;
mod files;
pub(crate) mod literature_analysis;
pub(crate) mod literature_attachments;
pub(crate) mod literature_bibliography;
pub(crate) mod literature_catalog;
pub(crate) mod literature_claims;
pub(crate) mod literature_concepts;
pub(crate) mod literature_context;
pub(crate) mod literature_derived_view;
pub(crate) mod literature_doi;
pub(crate) mod literature_doi_commands;
pub(crate) mod literature_duplicate_merge;
pub(crate) mod literature_duplicate_undo;
pub(crate) mod literature_duplicates;
pub(crate) mod literature_evidence;
pub(crate) mod literature_export;
pub(crate) mod literature_hbs;
pub(crate) mod literature_import_jobs;
pub(crate) mod literature_index;
pub(crate) mod literature_knowledge_promotion;
pub(crate) mod literature_lifecycle;
pub(crate) mod literature_matrix;
pub(crate) mod literature_metadata;
pub(crate) mod literature_nal_annotation;
pub(crate) mod literature_nal_bootstrap;
pub(crate) mod literature_nal_mutation;
pub(crate) mod literature_nal_query;
pub(crate) mod literature_pdf_import;
pub(crate) mod literature_reading_units;
pub(crate) mod literature_reference_records;
pub(crate) mod literature_retrieval;
pub(crate) mod literature_trace;
pub(crate) mod literature_trash_cleanup;
pub(crate) mod management;
pub(crate) mod metadata_http;
pub(crate) mod native;
pub(crate) mod reading_evidence;
mod scan;
pub(crate) mod writing_projects;

use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
};

const MAX_CATALOG_SNAPSHOTS: usize = 128;

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code}: {message}")]
pub(crate) struct RuntimePluginError {
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
}

impl RuntimePluginError {
    pub(crate) fn http_status(&self) -> u16 {
        match self.code {
            "runtime_plugin_package_unknown"
            | "runtime_plugin_resource_unknown"
            | "runtime_plugin_resource_missing" => 404,
            "runtime_plugin_resource_changed" => 409,
            "runtime_plugin_resource_forbidden" => 403,
            "runtime_plugin_resource_limit"
            | "runtime_plugin_package_limit"
            | "runtime_plugin_catalog_limit" => 413,
            "runtime_plugin_platform_unsupported" => 501,
            "runtime_plugin_path_invalid"
            | "runtime_plugin_package_key_invalid"
            | "runtime_plugin_root_invalid"
            | "runtime_plugin_manifest_invalid"
            | "runtime_plugin_entry_missing"
            | "runtime_plugin_resource_unsupported" => 400,
            _ => 500,
        }
    }

    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            package_key: None,
            resource: None,
        }
    }

    fn resource(mut self, resource: &str) -> Self {
        self.resource = Some(resource.to_owned());
        self
    }

    fn package(mut self, key: &str) -> Self {
        self.package_key = Some(key.to_owned());
        self
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimePluginPackage {
    pub package_key: String,
    pub manifest: serde_json::Value,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimePluginListing {
    pub packages: Vec<RuntimePluginPackage>,
    pub native_writing_preview: bool,
}

pub(crate) struct RuntimePluginResource {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

#[derive(Clone)]
struct FileSnapshot {
    real_path: PathBuf,
    sha256: [u8; 32],
}

#[derive(Clone)]
struct PackageSnapshot {
    controlled_root: PathBuf,
    directory: PathBuf,
    real_directory: PathBuf,
    files: BTreeMap<String, FileSnapshot>,
}

/// One Core-owned catalog, shared by IPC discovery and the resource protocol.
/// Construction is pure: an invalid/unavailable configured directory cannot
/// prevent Core startup. Existing snapshots survive subsequent discovery calls.
#[derive(Clone)]
pub(crate) struct RuntimePluginCatalog {
    root: Option<PathBuf>,
    bundled_root: Arc<OnceLock<PathBuf>>,
    snapshots: Arc<Mutex<BTreeMap<String, PackageSnapshot>>>,
    native_writing_preview: bool,
    management: Arc<OnceLock<management::ManagedPlugins>>,
    #[cfg(unix)]
    native_staging: Arc<Mutex<Option<Arc<native::staging::StagingSession>>>>,
}

impl RuntimePluginCatalog {
    pub(crate) fn new(root: Option<PathBuf>) -> Self {
        Self {
            root,
            bundled_root: Arc::new(OnceLock::new()),
            snapshots: Arc::new(Mutex::new(BTreeMap::new())),
            native_writing_preview: false,
            management: Arc::new(OnceLock::new()),
            #[cfg(unix)]
            native_staging: Arc::new(Mutex::new(None)),
        }
    }

    pub(crate) fn with_native_writing_preview(mut self, enabled: bool) -> Self {
        self.native_writing_preview = enabled;
        self
    }

    /// App resolves its packaged resources once. An explicit directory keeps
    /// precedence, including when it is unavailable or invalid.
    pub(crate) fn attach_bundled_root(&self, root: PathBuf) {
        self.bundled_root.get_or_init(|| root);
    }

    pub(crate) fn discover(&self) -> Result<RuntimePluginListing, RuntimePluginError> {
        let discovered = self.bundled_packages()?;
        let discovered = match self.management.get() {
            Some(manager) => manager.select(discovered)?,
            None => discovered,
        };
        let mut snapshots = self.snapshots.lock().map_err(|_| catalog_unavailable())?;
        let additional = discovered
            .iter()
            .map(|(package, _)| package.package_key.as_str())
            .filter(|key| !snapshots.contains_key(*key))
            .collect::<BTreeSet<_>>()
            .len();
        if snapshots.len() + additional > MAX_CATALOG_SNAPSHOTS {
            return Err(RuntimePluginError::new(
                "runtime_plugin_catalog_limit",
                "Core插件包目录最多保留128个内容快照；本次发现未接纳，已有快照保留。",
            ));
        }
        let mut packages = Vec::with_capacity(discovered.len());
        for (package, snapshot) in discovered {
            // A key is content based, so duplicate identical packages still go
            // to the SDK resolver as separate declarations; do not hide IDs.
            // A successful rescan refreshes a verified location for identical
            // content; retaining the first path would strand a renamed package.
            snapshots.insert(package.package_key.clone(), snapshot);
            packages.push(package);
        }
        Ok(RuntimePluginListing {
            packages,
            native_writing_preview: self.native_writing_preview,
        })
    }

    pub(crate) fn resource(
        &self,
        package_key: &str,
        relative_path: &str,
    ) -> Result<RuntimePluginResource, RuntimePluginError> {
        if package_key.len() != 64
            || !package_key
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(RuntimePluginError::new(
                "runtime_plugin_package_key_invalid",
                "插件包快照身份无效或尚未发现。",
            ));
        }
        files::relative_path(relative_path)?;
        let snapshot = self
            .snapshots
            .lock()
            .map_err(|_| catalog_unavailable())?
            .get(package_key)
            .cloned()
            .ok_or_else(|| {
                RuntimePluginError::new("runtime_plugin_package_unknown", "插件包快照尚未发现。")
                    .package(package_key)
            })?;
        let file = snapshot.files.get(relative_path).ok_or_else(|| {
            RuntimePluginError::new(
                "runtime_plugin_resource_unknown",
                "资源不在已发现的插件包快照中。",
            )
            .package(package_key)
            .resource(relative_path)
        })?;
        let result = (|| {
            let real_root = files::controlled_root(&snapshot.controlled_root)?;
            if real_root != snapshot.controlled_root
                || files::directory(&snapshot.directory, &real_root)? != snapshot.real_directory
            {
                return Err(files::changed(relative_path));
            }
            let (real_path, bytes) =
                files::read(&snapshot.directory, &snapshot.real_directory, relative_path)?;
            if real_path != file.real_path || files::digest(&bytes) != file.sha256 {
                return Err(files::changed(relative_path));
            }
            Ok(RuntimePluginResource {
                bytes,
                mime_type: files::mime_type(relative_path).into(),
            })
        })();
        result.map_err(|error: RuntimePluginError| error.package(package_key))
    }
}

fn catalog_unavailable() -> RuntimePluginError {
    RuntimePluginError::new(
        "runtime_plugin_catalog_unavailable",
        "插件包目录状态不可用。",
    )
}

#[tauri::command]
pub(crate) async fn list_runtime_plugin_packages(
    catalog: tauri::State<'_, Arc<RuntimePluginCatalog>>,
) -> Result<RuntimePluginListing, RuntimePluginError> {
    let catalog = catalog.inner().clone();
    tokio::task::spawn_blocking(move || catalog.discover())
        .await
        .map_err(|_| {
            RuntimePluginError::new(
                "runtime_plugin_worker_unavailable",
                "插件包目录读取任务未完成。",
            )
        })?
}

#[cfg(test)]
mod tests;

pub(crate) mod literature_browser_capture;
pub(crate) mod literature_reading_history;

pub(crate) mod literature_paper_confirmation;
