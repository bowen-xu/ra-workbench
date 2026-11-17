use super::{
    super::{PackageSnapshot, RuntimePluginError, RuntimePluginPackage, files, scan},
    store,
};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
};

pub(super) struct StagedPackage {
    _temporary: tempfile::TempDir,
    pub package: RuntimePluginPackage,
    pub snapshot: PackageSnapshot,
}

pub(super) fn stage(source: &Path, root: &Path) -> Result<StagedPackage, RuntimePluginError> {
    let parent = source.parent().ok_or_else(|| files::unsupported())?;
    let source_root = files::controlled_root(parent)?;
    let staging = store::directory(&root.join("staging"))?;
    let temporary = tempfile::tempdir_in(&staging).map_err(|e| files::io("插件暂存目录创建", e))?;
    let output = temporary.path().join("package");
    fs::create_dir(&output).map_err(|e| files::io("插件目录创建", e))?;
    if source
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
    {
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(files::unsupported)?;
        let (_, bytes) = files::read(parent, &source_root, name)?;
        unpack(&bytes, &output)?;
    } else {
        let directory = if source
            .file_name()
            .is_some_and(|name| name == "manifest.json")
        {
            parent
        } else {
            source
        };
        let controlled =
            files::controlled_root(directory.parent().ok_or_else(files::unsupported)?)?;
        let (_, snapshot) = scan::read_package(directory, &controlled)?;
        for (relative, original) in &snapshot.files {
            let (_, bytes) = files::read(directory, &snapshot.real_directory, relative)?;
            if files::digest(&bytes) != original.sha256 {
                return Err(files::changed(relative));
            }
            write(&output, relative, &bytes)?;
        }
    }
    let controlled = files::controlled_root(temporary.path())?;
    let package_directory = if output.join("manifest.json").is_file() {
        output
    } else {
        let mut entries = fs::read_dir(&output).map_err(|e| files::io("ZIP包根目录读取", e))?;
        let entry = entries
            .next()
            .transpose()
            .map_err(|e| files::io("ZIP包根目录读取", e))?
            .ok_or_else(|| {
                store::error("runtime_plugin_entry_missing", "插件包缺少 manifest.json。")
            })?;
        if entries.next().is_some() || !entry.path().join("manifest.json").is_file() {
            return Err(store::error(
                "runtime_plugin_entry_missing",
                "ZIP根目录或唯一顶层包目录须包含 manifest.json。",
            ));
        }
        entry.path()
    };
    let (package, snapshot) = scan::read_package(&package_directory, &controlled)?;
    identity(&package)?;
    Ok(StagedPackage {
        _temporary: temporary,
        package,
        snapshot,
    })
}

pub(super) fn identity(package: &RuntimePluginPackage) -> Result<&str, RuntimePluginError> {
    let id = package
        .manifest
        .get("id")
        .and_then(serde_json::Value::as_str)
        .filter(|id| store::valid_id(id))
        .ok_or_else(|| store::error("plugin_manifest_invalid", "插件 ID 无效。"))?;
    let version = package
        .manifest
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if version.len() > 128
        || !regex::Regex::new(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$")
            .map_err(|e| store::error("plugin_manifest_invalid", e.to_string()))?
            .is_match(version)
    {
        return Err(store::error("plugin_manifest_invalid", "插件版本号无效。"));
    }
    if let Some(native) = package.manifest.get("native") {
        if native.get("os").and_then(serde_json::Value::as_str) != Some(std::env::consts::OS)
            || native.get("arch").and_then(serde_json::Value::as_str)
                != Some(std::env::consts::ARCH)
        {
            return Err(store::error(
                "plugin_native_platform_incompatible",
                "插件原生包的平台或架构与当前应用不兼容。",
            ));
        }
    }
    Ok(id)
}

fn write(root: &Path, relative: &str, bytes: &[u8]) -> Result<(), RuntimePluginError> {
    files::relative_path(relative)?;
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| files::io("插件子目录创建", e))?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| files::io("插件资源暂存", e))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|e| files::io("插件资源落盘", e))
}

fn unpack(bytes: &[u8], output: &Path) -> Result<(), RuntimePluginError> {
    let invalid = |error| store::error("plugin_archive_invalid", format!("插件 ZIP 无效：{error}"));
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(invalid)?;
    if archive.len() > 4096 {
        return Err(store::error(
            "runtime_plugin_package_limit",
            "插件 ZIP 超过4096项限制。",
        ));
    }
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(invalid)?;
        let relative = entry.name().trim_end_matches('/');
        files::relative_path(relative)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000 | 0o040000))
        {
            return Err(files::unsupported().resource(relative));
        }
        if entry.is_dir() {
            fs::create_dir_all(output.join(relative)).map_err(|e| files::io("ZIP子目录创建", e))?;
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > 256 * 1024 * 1024 || entry.size() > files::MAX_RESOURCE_BYTES {
            return Err(store::error(
                "runtime_plugin_package_limit",
                "插件 ZIP 超过资源大小限制。",
            ));
        }
        let relative = relative.to_owned();
        let mut bytes = Vec::new();
        (&mut entry)
            .take(files::MAX_RESOURCE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| files::io("ZIP资源读取", e))?;
        if bytes.len() as u64 > files::MAX_RESOURCE_BYTES {
            return Err(files::unsupported().resource(&relative));
        }
        write(output, &relative, &bytes)?;
    }
    Ok(())
}
