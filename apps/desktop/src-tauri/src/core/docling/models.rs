//! Independent, immutable model bundles. Only a fully verified bundle becomes active.
#[path = "models/storage.rs"]
mod storage;
use super::error::DoclingError;
use fs2::FileExt;
use futures_util::StreamExt;
use research_docling_runtime::ModelManifest;
use serde::Serialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
pub use storage::ModelStorageReport;
use tokio::io::AsyncWriteExt;

#[derive(Clone)]
pub(crate) struct ModelResources {
    root: PathBuf,
    validator: Option<PathBuf>,
    progress: Arc<Mutex<ModelStatus>>,
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub state: String,
    pub installed_version: Option<String>,
    pub available_version: Option<String>,
    pub installed_identity: Option<String>,
    pub available_identity: Option<String>,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub last_error: Option<String>,
}

impl ModelResources {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self {
            root,
            validator: None,
            progress: Arc::new(Mutex::new(ModelStatus {
                state: "not_installed".into(),
                ..ModelStatus::default()
            })),
        }
    }
    pub(crate) fn native(root: PathBuf, binary: PathBuf) -> Self {
        Self {
            validator: Some(binary),
            ..Self::new(root)
        }
    }
    pub(crate) fn current(&self) -> Result<(ModelManifest, PathBuf), DoclingError> {
        let bytes = fs::read(self.root.join("active.json"))
            .map_err(|_| DoclingError::Models("尚未安装模型，请在设置中下载 FP32 模型".into()))?;
        let manifest: ModelManifest = serde_json::from_slice(&bytes)?;
        let name = manifest.directory_name().map_err(DoclingError::Models)?;
        let directory = self.root.join(name);
        let installed: ModelManifest =
            serde_json::from_slice(&fs::read(directory.join("manifest.json"))?)?;
        if installed.identity().map_err(DoclingError::Models)?
            != manifest.identity().map_err(DoclingError::Models)?
        {
            return Err(DoclingError::Models("模型包身份不一致，请重新下载".into()));
        }
        Ok((manifest, directory))
    }
    pub(crate) fn status(&self) -> ModelStatus {
        let mut status = self.progress.lock().map(|s| s.clone()).unwrap_or_default();
        if let Ok((manifest, _)) = self.current() {
            status.installed_version = Some(manifest.version.clone());
            status.installed_identity = manifest.identity().ok();
            if status.state == "not_installed" {
                status.state = "ready".into();
            }
        }
        status
    }
    fn update(&self, action: impl FnOnce(&mut ModelStatus)) {
        if let Ok(mut state) = self.progress.lock() {
            action(&mut state);
        }
    }
    pub(crate) async fn check(&self, url: &str) -> Result<ModelStatus, DoclingError> {
        let manifest = fetch_manifest(url).await?;
        let identity = manifest.identity().map_err(DoclingError::Models)?;
        self.update(|s| {
            s.available_version = Some(manifest.version);
            s.available_identity = Some(identity);
            s.last_error = None;
        });
        Ok(self.status())
    }
    pub(crate) async fn install(&self, url: &str) -> Result<ModelStatus, DoclingError> {
        fs::create_dir_all(&self.root)?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.root.join("download.lock"))?;
        lock.try_lock_exclusive()
            .map_err(|_| DoclingError::Models("另一个客户端正在下载模型".into()))?;
        self.update(|s| {
            s.state = "downloading".into();
            s.last_error = None;
            s.downloaded_bytes = 0;
            s.total_bytes = 0;
        });
        let result = self.install_inner(url).await;
        drop(lock);
        match result {
            Ok(()) => {
                self.update(|s| s.state = "ready".into());
                Ok(self.status())
            }
            Err(error) => {
                self.update(|s| {
                    s.state = "failed".into();
                    s.last_error = Some(error.to_string());
                });
                Err(error)
            }
        }
    }
    async fn install_inner(&self, url: &str) -> Result<(), DoclingError> {
        let manifest = fetch_manifest(url).await?;
        let identity = manifest.identity().map_err(DoclingError::Models)?;
        let name = manifest.directory_name().map_err(DoclingError::Models)?;
        self.update(|s| {
            s.available_version = Some(manifest.version.clone());
            s.available_identity = Some(identity);
            s.total_bytes = manifest.files.iter().map(|f| f.size_bytes).sum();
        });
        let staging = tempfile::Builder::new()
            .prefix(".download-")
            .tempdir_in(&self.root)?;
        let base = validated_url(url)?;
        let client = model_client()?;
        for file in &manifest.files {
            let destination = staging.path().join(&file.path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            let resource = base
                .join(&format!("{name}/{}", file.path))
                .map_err(|e| DoclingError::Models(e.to_string()))?;
            let response = client.get(resource).send().await?.error_for_status()?;
            let mut output = tokio::fs::File::create(&destination).await?;
            let mut stream = response.bytes_stream();
            let mut received = 0u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                received += chunk.len() as u64;
                if received > file.size_bytes {
                    return Err(DoclingError::Models("模型下载超过清单大小".into()));
                }
                output.write_all(&chunk).await?;
                self.update(|s| s.downloaded_bytes += chunk.len() as u64);
            }
            output.sync_all().await?;
        }
        self.update(|s| s.state = "verifying".into());
        let path = staging.path().to_owned();
        let verification = manifest.clone();
        tokio::task::spawn_blocking(move || verification.verify(&path))
            .await
            .map_err(|e| DoclingError::Models(e.to_string()))?
            .map_err(DoclingError::Models)?;
        let bytes = serde_json::to_vec_pretty(&manifest)?;
        fs::write(staging.path().join("manifest.json"), &bytes)?;
        if let Some(binary) = &self.validator {
            let service_root = self
                .root
                .parent()
                .ok_or_else(|| DoclingError::Models("模型目录缺少服务根目录".into()))?;
            let executable = super::executable::prepare_executable(binary, service_root)?;
            let mut command = tokio::process::Command::new(executable);
            super::launcher::configure_model_environment(&mut command, staging.path());
            command
                .args(["--validate-models", "1", "--model-directory"])
                .arg(staging.path())
                .current_dir(staging.path())
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true);
            let status = tokio::time::timeout(Duration::from_secs(90), command.status())
                .await
                .map_err(|_| DoclingError::Models("模型加载验证超时，保留旧模型".into()))??;
            if !status.success() {
                return Err(DoclingError::Models(
                    "模型无法由当前 FP32 引擎加载，保留旧模型".into(),
                ));
            }
        }
        let destination = self.root.join(name);
        if destination.exists() {
            let existing = manifest.clone();
            let target = destination.clone();
            let verified = tokio::task::spawn_blocking(move || existing.verify(&target))
                .await
                .map_err(|e| DoclingError::Models(e.to_string()))?;
            if verified.is_err() {
                let backup = self.root.join(format!(".corrupt-{}", uuid::Uuid::new_v4()));
                fs::rename(&destination, &backup)?;
                if let Err(error) = fs::rename(staging.path(), &destination) {
                    fs::rename(&backup, &destination)?;
                    return Err(error.into());
                }
                // Keep the damaged copy for diagnosis; never make it active again.
            }
        } else {
            fs::rename(staging.path(), &destination)?;
        }
        atomic_write(&self.root.join("active.json"), &bytes)?;
        Ok(())
    }
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), DoclingError> {
    let parent = path
        .parent()
        .ok_or_else(|| DoclingError::Models("模型路径无效".into()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|e| DoclingError::File(e.error))?;
    Ok(())
}
pub(crate) fn validated_url(value: &str) -> Result<reqwest::Url, DoclingError> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| DoclingError::Models("模型更新源 URL 无效".into()))?;
    if !(url.scheme() == "https" || (url.scheme() == "http" && url.host_str() == Some("127.0.0.1")))
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.path().ends_with("/latest.json")
    {
        return Err(DoclingError::Models(
            "模型更新源须为 HTTPS 或本机 127.0.0.1 HTTP 的 latest.json".into(),
        ));
    }
    Ok(url)
}
fn model_client() -> Result<reqwest::Client, DoclingError> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(1800))
        .build()?)
}
async fn fetch_manifest(url: &str) -> Result<ModelManifest, DoclingError> {
    let response = model_client()?
        .get(validated_url(url)?)
        .timeout(Duration::from_secs(15))
        .send()
        .await?
        .error_for_status()?;
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk?);
        if bytes.len() > 64 * 1024 {
            return Err(DoclingError::Models("模型清单过大".into()));
        }
    }
    let manifest: ModelManifest = serde_json::from_slice(&bytes)?;
    manifest.validate().map_err(DoclingError::Models)?;
    Ok(manifest)
}

#[cfg(test)]
#[path = "models/tests.rs"]
mod tests;
