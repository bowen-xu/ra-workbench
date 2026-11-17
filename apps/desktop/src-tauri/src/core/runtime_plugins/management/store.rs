use super::super::{RuntimePluginError, files};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Selection {
    pub package_key: Option<String>,
    pub enabled: bool,
    pub removed: bool,
    #[serde(default)]
    pub manifest: Option<serde_json::Value>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Registry {
    pub format_version: u32,
    pub plugins: BTreeMap<String, Selection>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            format_version: 1,
            plugins: BTreeMap::new(),
        }
    }
}

pub(super) struct RegistryGuard {
    pub state: Registry,
    pub revision: String,
    root: PathBuf,
    _lock: fs::File,
}

pub(super) fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id != "core"
        && id.as_bytes()[0].is_ascii_lowercase()
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}
pub(super) fn error(code: &'static str, message: impl Into<String>) -> RuntimePluginError {
    RuntimePluginError::new(code, message)
}

pub(super) fn directory(path: &Path) -> Result<PathBuf, RuntimePluginError> {
    fs::create_dir_all(path).map_err(|e| files::io("管理目录创建", e))?;
    if fs::symlink_metadata(path)
        .map_err(|e| files::io("管理目录读取", e))?
        .file_type()
        .is_symlink()
    {
        return Err(files::unsupported());
    }
    files::controlled_root(path)
}

fn lock_file(path: &Path) -> Result<fs::File, RuntimePluginError> {
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(nix::libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|e| files::io("管理锁打开", e))?;
    if !file
        .metadata()
        .map_err(|e| files::io("管理锁检查", e))?
        .is_file()
    {
        return Err(files::unsupported());
    }
    Ok(file)
}

pub(super) fn registry(root: &Path) -> Result<RegistryGuard, RuntimePluginError> {
    let root = directory(root)?;
    let lock = lock_file(&root.join("registry.lock"))?;
    lock.lock_exclusive()
        .map_err(|e| files::io("管理锁获取", e))?;
    let path = root.join("registry.json");
    let exists = path
        .try_exists()
        .map_err(|e| files::io("安装记录定位", e))?;
    let bytes = if exists {
        files::read(&root, &root, "registry.json")?.1
    } else {
        Vec::new()
    };
    if bytes.len() > 1024 * 1024 {
        return Err(error(
            "plugin_registry_invalid",
            "插件安装记录超过大小限制。",
        ));
    }
    let state: Registry = if !exists {
        Registry::default()
    } else {
        serde_json::from_slice(&bytes).map_err(|_| {
            error(
                "plugin_registry_invalid",
                "插件安装记录无效，原文件已保留。",
            )
        })?
    };
    if state.format_version != 1
        || state.plugins.len() > 128
        || state.plugins.iter().any(|(id, row)| {
            !valid_id(id)
                || row
                    .package_key
                    .as_deref()
                    .is_some_and(|key| !valid_key(key))
        })
    {
        return Err(error(
            "plugin_registry_invalid",
            "插件安装记录的版本、身份或数量无效。",
        ));
    }
    let revision = format!("{:x}", sha2::Sha256::digest(&bytes));
    Ok(RegistryGuard {
        state,
        revision,
        root,
        _lock: lock,
    })
}
use sha2::Digest;

impl RegistryGuard {
    pub fn expect(&self, revision: &str) -> Result<(), RuntimePluginError> {
        if revision != self.revision {
            return Err(error(
                "plugin_registry_conflict",
                "插件配置已被其他窗口或应用实例修改，请刷新后重试。",
            ));
        }
        Ok(())
    }
    pub fn save(&self) -> Result<(), RuntimePluginError> {
        let bytes = serde_json::to_vec(&self.state)
            .map_err(|e| error("plugin_registry_write_failed", e.to_string()))?;
        if bytes.len() > 1024 * 1024 || self.state.plugins.len() > 128 {
            return Err(error(
                "plugin_registry_limit",
                "插件安装记录超过128项或1 MiB限制，本次更改未提交。",
            ));
        }
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)
            .map_err(|e| files::io("安装记录暂存", e))?;
        temporary
            .write_all(&bytes)
            .map_err(|e| files::io("安装记录写入", e))?;
        temporary
            .flush()
            .map_err(|e| files::io("安装记录刷新", e))?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|e| files::io("安装记录落盘", e))?;
        temporary
            .persist(self.root.join("registry.json"))
            .map_err(|e| files::io("安装记录提交", e.error))?;
        #[cfg(unix)]
        fs::File::open(&self.root)
            .and_then(|file| file.sync_all())
            .map_err(|e| files::io("安装目录落盘", e))?;
        Ok(())
    }
}

pub(super) fn lease(root: &Path, key: &str) -> Result<fs::File, RuntimePluginError> {
    let root = directory(&root.join("leases"))?;
    let file = lock_file(&root.join(key))?;
    FileExt::lock_shared(&file).map_err(|e| files::io("插件使用锁获取", e))?;
    Ok(file)
}

// Running processes retain a shared lease. Uninstall removes selection now;
// obsolete bytes are reclaimed only after every process has released that package.
pub(super) fn collect(root: &Path, state: &Registry) -> Result<(), RuntimePluginError> {
    let packages = directory(&root.join("packages"))?;
    let leases = directory(&root.join("leases"))?;
    for entry in fs::read_dir(&packages).map_err(|e| files::io("旧包枚举", e))? {
        let entry = entry.map_err(|e| files::io("旧包读取", e))?;
        let key = entry.file_name().to_string_lossy().into_owned();
        if !valid_key(&key)
            || state
                .plugins
                .values()
                .any(|row| !row.removed && row.package_key.as_deref() == Some(&key))
        {
            continue;
        }
        let lock = lock_file(&leases.join(&key))?;
        if FileExt::try_lock_exclusive(&lock).is_err() {
            continue;
        }
        files::directory(&entry.path(), &packages)?;
        fs::remove_dir_all(entry.path()).map_err(|e| files::io("已卸载包回收", e))?;
    }
    Ok(())
}
