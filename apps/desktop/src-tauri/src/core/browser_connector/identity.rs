//! Durable receiver identity and import leases; no user-visible pairing secret.
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    sync::RwLock,
};

use sha2::{Digest, Sha256};
use uuid::Uuid;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

pub(super) struct ReceiverIdentity {
    id: String,
    imports: RwLock<()>,
}

impl ReceiverIdentity {
    pub(super) fn open(directory: &Path) -> Result<Self, String> {
        fs::create_dir_all(directory).map_err(|_| "无法创建浏览器连接配置".to_owned())?;
        let metadata =
            fs::symlink_metadata(directory).map_err(|_| "无法读取浏览器连接配置".to_owned())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("浏览器连接配置目录不安全".to_owned());
        }
        #[cfg(unix)]
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法保护浏览器连接配置".to_owned())?;
        // Preserve the first development version's filename and hash so its
        // capture receipts remain addressable after automatic discovery upgrades.
        let path = directory.join("pairing-code");
        let seed = match fs::symlink_metadata(&path) {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 64
                {
                    return Err("浏览器接收端身份文件不安全".to_owned());
                }
                #[cfg(unix)]
                if metadata.permissions().mode() & 0o077 != 0 {
                    return Err("浏览器接收端身份文件权限不安全".to_owned());
                }
                let mut options = OpenOptions::new();
                options.read(true);
                #[cfg(unix)]
                options.custom_flags(nix::libc::O_NOFOLLOW);
                let file = options
                    .open(&path)
                    .map_err(|_| "无法读取浏览器接收端身份".to_owned())?;
                let mut seed = String::new();
                file.take(65)
                    .read_to_string(&mut seed)
                    .map_err(|_| "无法读取浏览器接收端身份".to_owned())?;
                seed
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let seed = new_connection_token();
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                options.mode(0o600);
                let mut file = options
                    .open(&path)
                    .map_err(|_| "无法创建浏览器接收端身份".to_owned())?;
                file.write_all(seed.as_bytes())
                    .and_then(|()| file.sync_all())
                    .map_err(|_| "无法保存浏览器接收端身份".to_owned())?;
                #[cfg(unix)]
                fs::File::open(directory)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| "浏览器接收端身份尚未可靠保存".to_owned())?;
                seed
            }
            Err(_) => return Err("无法读取浏览器接收端身份".to_owned()),
        };
        if seed.len() != 64
            || !seed
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err("浏览器接收端身份格式损坏".to_owned());
        }
        Ok(Self {
            id: format!("{:x}", Sha256::digest(seed.as_bytes())),
            imports: RwLock::new(()),
        })
    }

    pub(super) fn id(&self) -> &str {
        &self.id
    }

    pub(super) fn with_scope<T>(&self, operation: impl FnOnce(&str) -> T) -> Option<T> {
        let _lease = self.imports.read().ok()?;
        Some(operation(&self.id))
    }

    pub(super) fn wait_for_imports(&self) -> Result<(), String> {
        let _lease = self
            .imports
            .write()
            .map_err(|_| "浏览器采集尚未完成，请重试退出".to_owned())?;
        Ok(())
    }
}

pub(super) fn new_connection_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receiver_scope_survives_reopen_and_preserves_the_legacy_receipt_namespace() {
        let directory = tempfile::tempdir().expect("directory");
        let identity = ReceiverIdentity::open(directory.path()).expect("identity");
        let seed = fs::read(directory.path().join("pairing-code")).expect("private identity seed");
        assert_eq!(identity.id(), format!("{:x}", Sha256::digest(&seed)));
        assert_eq!(
            ReceiverIdentity::open(directory.path())
                .expect("reopen")
                .id(),
            identity.id()
        );
        assert_eq!(
            identity.with_scope(str::to_owned),
            Some(identity.id().to_owned())
        );
    }

    #[test]
    fn damaged_identity_is_not_silently_replaced() {
        let directory = tempfile::tempdir().expect("directory");
        ReceiverIdentity::open(directory.path()).expect("identity");
        fs::write(directory.path().join("pairing-code"), "invalid").expect("fixture");
        assert!(ReceiverIdentity::open(directory.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_public_identity_seeds_are_rejected() {
        let directory = tempfile::tempdir().expect("directory");
        ReceiverIdentity::open(directory.path()).expect("identity");
        let path = directory.path().join("pairing-code");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("permissions");
        assert!(ReceiverIdentity::open(directory.path()).is_err());
        fs::remove_file(&path).expect("remove fixture");
        let target = directory.path().join("other");
        fs::write(&target, "a".repeat(64)).expect("fixture");
        std::os::unix::fs::symlink(target, path).expect("link");
        assert!(ReceiverIdentity::open(directory.path()).is_err());
    }
}
