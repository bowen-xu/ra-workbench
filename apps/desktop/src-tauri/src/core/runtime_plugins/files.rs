use super::RuntimePluginError;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub(super) const MAX_RESOURCE_BYTES: u64 = 64 * 1024 * 1024;

pub(super) fn relative_path(value: &str) -> Result<(), RuntimePluginError> {
    if value.is_empty()
        || value.starts_with('/')
        || value
            .chars()
            .any(|character| character.is_ascii_control() || ":\\%?#".contains(character))
        || value
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(RuntimePluginError::new(
            "runtime_plugin_path_invalid",
            "插件资源必须使用未编码的规范相对POSIX路径。",
        ));
    }
    Ok(())
}

pub(super) fn controlled_root(root: &Path) -> Result<PathBuf, RuntimePluginError> {
    if !root.is_absolute()
        || root
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(RuntimePluginError::new(
            "runtime_plugin_root_invalid",
            "插件包目录必须是显式配置的绝对目录。",
        ));
    }
    let real = root.canonicalize().map_err(|error| io("目录定位", error))?;
    if !real.is_dir() {
        return Err(RuntimePluginError::new(
            "runtime_plugin_root_invalid",
            "配置的插件包位置不是目录。",
        ));
    }
    Ok(real)
}

pub(super) fn directory(
    directory: &Path,
    controlled_root: &Path,
) -> Result<PathBuf, RuntimePluginError> {
    let metadata = fs::symlink_metadata(directory).map_err(|error| io("包目录读取", error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(unsupported());
    }
    let real = directory
        .canonicalize()
        .map_err(|error| io("包目录定位", error))?;
    if !real.starts_with(controlled_root) || real == controlled_root {
        return Err(unsupported());
    }
    Ok(real)
}

fn checked_file(
    directory: &Path,
    real_directory: &Path,
    relative: &str,
) -> Result<PathBuf, RuntimePluginError> {
    relative_path(relative)?;
    let mut path = directory.to_owned();
    let segments: Vec<_> = relative.split('/').collect();
    for (index, segment) in segments.iter().enumerate() {
        path.push(segment);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| io("资源状态读取", error).resource(relative))?;
        if metadata.file_type().is_symlink()
            || if index + 1 == segments.len() {
                !metadata.is_file()
            } else {
                !metadata.is_dir()
            }
        {
            return Err(unsupported().resource(relative));
        }
    }
    let real = path
        .canonicalize()
        .map_err(|error| io("资源定位", error).resource(relative))?;
    if !real.starts_with(real_directory) || real == real_directory {
        return Err(unsupported().resource(relative));
    }
    Ok(real)
}

pub(super) fn read(
    directory: &Path,
    real_directory: &Path,
    relative: &str,
) -> Result<(PathBuf, Vec<u8>), RuntimePluginError> {
    let real = checked_file(directory, real_directory, relative)?;
    let file = open_file(real_directory, relative)?;
    let mut bytes = Vec::new();
    file.take(MAX_RESOURCE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| io("资源读取", error).resource(relative))?;
    if bytes.len() as u64 > MAX_RESOURCE_BYTES {
        return Err(RuntimePluginError::new(
            "runtime_plugin_resource_limit",
            "单个插件构建资源超过64 MiB限制。",
        )
        .resource(relative));
    }
    if checked_file(directory, real_directory, relative)? != real {
        return Err(changed(relative));
    }
    Ok((real, bytes))
}

fn open_file(real_directory: &Path, relative: &str) -> Result<fs::File, RuntimePluginError> {
    use crate::agent::instructions::{InstructionReadError, directory::Directory};

    relative_path(relative)?;
    let error = |cause| match cause {
        InstructionReadError::InvalidScope => unsupported().resource(relative),
        InstructionReadError::UnsupportedPlatform => RuntimePluginError::new(
            "runtime_plugin_platform_unsupported",
            "此平台尚未提供插件包资源所需的原子受控目录读取。",
        ),
        cause => RuntimePluginError::new(
            "runtime_plugin_filesystem_unavailable",
            format!("插件包受控资源打开失败：{cause}"),
        )
        .resource(relative),
    };
    // Reuse the Core primitive already used by workspace disk operations:
    // every directory is opened relative to a pinned fd with DIRECTORY /
    // NOFOLLOW; the leaf uses NOFOLLOW / NONBLOCK and fstat regular-file checks.
    // Non-Unix Directory explicitly returns UnsupportedPlatform.
    let mut directory = Directory::root(real_directory).map_err(error)?;
    let parts: Vec<_> = relative.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        directory = directory.child(Path::new(part)).map_err(error)?;
    }
    directory
        .file(parts[parts.len() - 1])
        .map_err(error)?
        .ok_or_else(|| {
            RuntimePluginError::new(
                "runtime_plugin_resource_missing",
                "插件构建资源不存在或不是常规文件。",
            )
            .resource(relative)
        })
}

#[cfg(all(test, unix))]
mod atomic_open_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn a_leaf_swapped_after_path_validation_cannot_be_opened_through_a_symlink() {
        let package = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(package.path().join("entry.js"), "original").unwrap();
        fs::write(outside.path().join("secret.js"), "outside").unwrap();
        let root = package.path().canonicalize().unwrap();
        checked_file(package.path(), &root, "entry.js").unwrap();
        fs::remove_file(package.path().join("entry.js")).unwrap();
        symlink(
            outside.path().join("secret.js"),
            package.path().join("entry.js"),
        )
        .unwrap();
        assert_eq!(
            open_file(&root, "entry.js").unwrap_err().code,
            "runtime_plugin_resource_unsupported"
        );
    }

    #[test]
    fn an_ancestor_swapped_after_path_validation_cannot_redirect_the_open() {
        let package = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(package.path().join("assets")).unwrap();
        fs::write(package.path().join("assets/chunk.js"), "original").unwrap();
        fs::write(outside.path().join("chunk.js"), "outside").unwrap();
        let root = package.path().canonicalize().unwrap();
        checked_file(package.path(), &root, "assets/chunk.js").unwrap();
        fs::rename(
            package.path().join("assets"),
            package.path().join("original-assets"),
        )
        .unwrap();
        symlink(outside.path(), package.path().join("assets")).unwrap();
        assert_eq!(
            open_file(&root, "assets/chunk.js").unwrap_err().code,
            "runtime_plugin_resource_unsupported"
        );
    }
}

pub(super) fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub(super) fn changed(relative: &str) -> RuntimePluginError {
    RuntimePluginError::new(
        "runtime_plugin_resource_changed",
        "插件包资源已改变，请重新发现并核对包后再加载。",
    )
    .resource(relative)
}

pub(super) fn unsupported() -> RuntimePluginError {
    RuntimePluginError::new(
        "runtime_plugin_resource_unsupported",
        "插件包仅支持受控目录内的常规文件和目录，不支持符号链接或特殊文件。",
    )
}

pub(super) fn io(operation: &str, error: std::io::Error) -> RuntimePluginError {
    let code = match error.kind() {
        std::io::ErrorKind::NotFound => "runtime_plugin_resource_missing",
        std::io::ErrorKind::PermissionDenied => "runtime_plugin_resource_forbidden",
        _ => "runtime_plugin_filesystem_unavailable",
    };
    RuntimePluginError::new(code, format!("插件包{operation}失败：{error}"))
}

pub(super) fn mime_type(relative: &str) -> &'static str {
    match relative.rsplit('.').next().unwrap_or("") {
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json; charset=utf-8",
        "html" => "text/html; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}
