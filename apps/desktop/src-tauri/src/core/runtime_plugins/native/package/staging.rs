use super::*;
#[cfg(unix)]
use std::{fs, io::Write};

pub(super) fn stage_bytes(
    catalog: &RuntimePluginCatalog,
    bytes: &[u8],
    profile: NativeProfile,
) -> NativeResult<StagedHost> {
    #[cfg(not(unix))]
    {
        let _ = (catalog, bytes);
        Err(NativeHostError::for_profile(
            profile,
            "plugin_host_platform_unsupported",
            "此平台尚未提供原生插件资源执行。",
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let io = |error: std::io::Error| {
            NativeHostError::for_profile(
                profile,
                "plugin_host_stage_failed",
                format!("原生插件私有副本准备失败：{:?}", error.kind()),
            )
        };
        let session = super::super::staging::for_catalog(catalog).map_err(|error| {
            NativeHostError::for_profile(
                profile,
                "plugin_host_stage_failed",
                format!("原生插件临时拥有者恢复失败：{:?}", error.kind()),
            )
        })?;
        let directory = tempfile::Builder::new()
            .prefix("research-native-host-")
            .tempdir_in(session.path())
            .map_err(io)?;
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).map_err(io)?;
        let executable = directory.path().join(profile.executable_name());
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&executable)
            .map_err(io)?;
        file.write_all(bytes).map_err(io)?;
        file.sync_all().map_err(io)?;
        file.set_permissions(fs::Permissions::from_mode(0o500))
            .map_err(io)?;
        drop(file);
        if crate::runtime_plugins::files::digest(&fs::read(&executable).map_err(io)?)
            != crate::runtime_plugins::files::digest(bytes)
        {
            return Err(NativeHostError::for_profile(
                profile,
                "plugin_host_resource_changed",
                "原生插件私有副本与已验证字节不一致。",
            ));
        }
        Ok(StagedHost {
            directory,
            executable,
            input_directory: if profile != NativeProfile::WritingPreview {
                let input = tempfile::Builder::new()
                    .prefix("research-native-input-")
                    .tempdir_in(session.path())
                    .map_err(io)?;
                fs::set_permissions(input.path(), fs::Permissions::from_mode(0o700)).map_err(io)?;
                Some(input)
            } else {
                None
            },
            profile,
            _session: session,
        })
    }
}
