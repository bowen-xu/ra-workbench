use super::{FileSnapshot, PackageSnapshot, RuntimePluginError, RuntimePluginPackage, files};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, path::Path};

const MAX_PACKAGE_ENTRIES: usize = 4096;
const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_DIRECTORY_DEPTH: usize = 32;
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_PACKAGE_DIRECTORIES: usize = 64;

pub(super) fn discover(
    root: &Path,
) -> Result<Vec<(RuntimePluginPackage, PackageSnapshot)>, RuntimePluginError> {
    let controlled_root = files::controlled_root(root)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(root).map_err(|error| files::io("目录枚举", error))? {
        let entry = entry.map_err(|error| files::io("目录条目读取", error))?;
        let kind = entry
            .file_type()
            .map_err(|error| files::io("包目录状态读取", error))?;
        if kind.is_symlink() {
            return Err(files::unsupported());
        }
        if !kind.is_dir() {
            continue;
        }
        if entries.len() >= MAX_PACKAGE_DIRECTORIES {
            return Err(RuntimePluginError::new(
                "runtime_plugin_catalog_limit",
                "受控目录最多包含64个插件包目录；本次发现未接纳，已有快照保留。",
            ));
        }
        entries.push(entry);
    }
    entries.sort_by_key(|entry| entry.file_name());
    let mut output = Vec::new();
    for entry in entries {
        let directory = entry.path();
        output.push(
            read_package(&directory, &controlled_root).map_err(|mut error| {
                error.message = format!(
                    "插件包「{}」：{}",
                    entry.file_name().to_string_lossy(),
                    error.message
                );
                error
            })?,
        );
    }
    Ok(output)
}

fn read_package(
    directory: &Path,
    controlled_root: &Path,
) -> Result<(RuntimePluginPackage, PackageSnapshot), RuntimePluginError> {
    let real_directory = files::directory(directory, controlled_root)?;
    let (_, manifest_bytes) = files::read(directory, &real_directory, "manifest.json")?;
    if manifest_bytes.len() > MAX_MANIFEST_BYTES {
        return Err(RuntimePluginError::new(
            "runtime_plugin_manifest_invalid",
            "插件包manifest超过1 MiB限制。",
        ));
    }
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        RuntimePluginError::new(
            "runtime_plugin_manifest_invalid",
            format!("插件包manifest不是有效JSON：{error}"),
        )
    })?;
    let mut snapshot = PackageSnapshot {
        controlled_root: controlled_root.to_owned(),
        directory: directory.to_owned(),
        real_directory,
        files: BTreeMap::new(),
    };
    let mut total = 0;
    let mut remaining = MAX_PACKAGE_ENTRIES;
    collect(&mut snapshot, "", 0, &mut total, &mut remaining)?;
    if snapshot.files.get("manifest.json").map(|file| file.sha256)
        != Some(files::digest(&manifest_bytes))
    {
        return Err(files::changed("manifest.json"));
    }
    let entry = manifest.get("entry").and_then(|value| value.as_str());
    if let Some(entry) = entry {
        declared_resource(&snapshot, entry)?;
    } else if manifest
        .get("formatVersion")
        .and_then(|value| value.as_u64())
        != Some(2)
        || manifest.get("native").is_none()
    {
        return Err(RuntimePluginError::new(
            "runtime_plugin_entry_missing",
            "插件包manifest缺少构建入口路径。",
        ));
    }
    if manifest
        .get("formatVersion")
        .and_then(|value| value.as_u64())
        == Some(2)
        && let Some(native) = manifest.get("native")
    {
        let entry = native
            .get("entry")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                RuntimePluginError::new(
                    "runtime_plugin_entry_missing",
                    "原生插件缺少构建入口路径。",
                )
            })?;
        declared_resource(&snapshot, entry)?;
    }
    // Only filesystem-bearing fields are inspected here. API versions,
    // IDs, dependency graphs and other manifest semantics belong to SDK.
    if let Some(styles) = manifest.get("styles") {
        let styles = styles.as_array().ok_or_else(|| {
            RuntimePluginError::new(
                "runtime_plugin_path_invalid",
                "插件包styles必须是资源路径数组。",
            )
        })?;
        for style in styles {
            let path = style.as_str().ok_or_else(|| {
                RuntimePluginError::new("runtime_plugin_path_invalid", "插件样式必须使用资源路径。")
            })?;
            declared_resource(&snapshot, path)?;
        }
    }
    let mut key = Sha256::new();
    key.update(b"research-assistant-runtime-package-v1\0");
    for (path, file) in &snapshot.files {
        key.update((path.len() as u64).to_be_bytes());
        key.update(path.as_bytes());
        key.update(file.sha256);
    }
    Ok((
        RuntimePluginPackage {
            package_key: format!("{:x}", key.finalize()),
            manifest,
        },
        snapshot,
    ))
}

fn declared_resource(snapshot: &PackageSnapshot, path: &str) -> Result<(), RuntimePluginError> {
    files::relative_path(path)?;
    if !snapshot.files.contains_key(path) {
        return Err(RuntimePluginError::new(
            "runtime_plugin_resource_missing",
            "插件包声明的构建资源不存在或不是常规文件。",
        )
        .resource(path));
    }
    Ok(())
}

fn collect(
    snapshot: &mut PackageSnapshot,
    relative: &str,
    depth: usize,
    total: &mut u64,
    remaining: &mut usize,
) -> Result<(), RuntimePluginError> {
    if depth > MAX_DIRECTORY_DEPTH {
        return Err(limit());
    }
    let path = snapshot.directory.join(relative);
    for entry in fs::read_dir(&path).map_err(|error| files::io("资源目录枚举", error))? {
        let entry = entry.map_err(|error| files::io("资源条目读取", error))?;
        *remaining = remaining.checked_sub(1).ok_or_else(limit)?;
        let name = entry.file_name().into_string().map_err(|_| {
            RuntimePluginError::new(
                "runtime_plugin_path_invalid",
                "插件包资源名必须是UTF-8路径。",
            )
        })?;
        let child = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        files::relative_path(&child)?;
        let kind = entry
            .file_type()
            .map_err(|error| files::io("资源状态读取", error))?;
        if kind.is_symlink() {
            return Err(files::unsupported().resource(&child));
        }
        if kind.is_dir() {
            collect(snapshot, &child, depth + 1, total, remaining)?;
        } else if kind.is_file() {
            let (real_path, bytes) =
                files::read(&snapshot.directory, &snapshot.real_directory, &child)?;
            *total = total.saturating_add(bytes.len() as u64);
            if *total > MAX_PACKAGE_BYTES {
                return Err(limit());
            }
            snapshot.files.insert(
                child,
                FileSnapshot {
                    real_path,
                    sha256: files::digest(&bytes),
                },
            );
        } else {
            return Err(files::unsupported().resource(&child));
        }
    }
    Ok(())
}

fn limit() -> RuntimePluginError {
    RuntimePluginError::new(
        "runtime_plugin_package_limit",
        "插件包超过4096个资源条目、256 MiB总量或32层目录限制。",
    )
}
