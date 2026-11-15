use super::*;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStorageReport {
    pub total_bytes: u64,
    pub installed_version: Option<String>,
    pub downloading: bool,
}

fn size(path: &Path) -> Result<u64, DoclingError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(DoclingError::Models(
            "模型目录含符号链接，无法安全清理".into(),
        ));
    }
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    let mut total = 0;
    for entry in fs::read_dir(path)? {
        total += size(&entry?.path())?;
    }
    Ok(total)
}

impl ModelResources {
    pub(crate) fn storage_report(&self) -> Result<ModelStorageReport, DoclingError> {
        let total_bytes = match fs::symlink_metadata(&self.root) {
            Ok(_) => size(&self.root)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
            Err(error) => return Err(error.into()),
        };
        let downloading = if self.root.exists() {
            let lock = self.removal_lock_file()?;
            match lock.try_lock_exclusive() {
                Ok(()) => false,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => true,
                Err(error) => return Err(error.into()),
            }
        } else {
            false
        };
        Ok(ModelStorageReport {
            total_bytes,
            installed_version: self.current().ok().map(|(m, _)| m.version),
            downloading,
        })
    }

    fn removal_lock_file(&self) -> Result<fs::File, DoclingError> {
        Ok(fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("download.lock"))?)
    }

    pub(crate) fn begin_removal(&self) -> Result<fs::File, DoclingError> {
        fs::create_dir_all(&self.root)?;
        let lock = self.removal_lock_file()?;
        lock.try_lock_exclusive().map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                DoclingError::Models("模型正在下载或清理，请稍后重试".into())
            } else {
                error.into()
            }
        })?;
        Ok(lock)
    }

    pub(crate) fn remove_models(&self) -> Result<ModelStorageReport, DoclingError> {
        let usage = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("usage.lock"))?;
        usage.try_lock_exclusive().map_err(|error| {
            if error.kind() == std::io::ErrorKind::WouldBlock {
                DoclingError::Models(
                    "模型仍被本地服务使用，请等待任务完成或其他客户端断开后重试".into(),
                )
            } else {
                error.into()
            }
        })?;
        // Validate the entire tree before deleting anything. Lock files retain
        // their inode so other processes cannot evade cross-process exclusion.
        size(&self.root)?;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if ["download.lock", "usage.lock"]
                .iter()
                .any(|name| entry.file_name() == *name)
            {
                continue;
            }
            if entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
            } else {
                fs::remove_file(entry.path())?;
            }
        }
        self.update(|state| {
            *state = ModelStatus {
                state: "not_installed".into(),
                ..ModelStatus::default()
            }
        });
        Ok(ModelStorageReport::default())
    }
}
