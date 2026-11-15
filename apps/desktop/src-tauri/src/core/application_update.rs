//! Application-owned controls around the official signed updater future.
use serde::Serialize;
use std::{
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::watch;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    revision: u64,
    phase: &'static str,
    version: Option<String>,
    downloaded: u64,
    total: Option<u64>,
    ready_to_install: bool,
    message: String,
}
impl Default for UpdateStatus {
    fn default() -> Self {
        Self {
            revision: 0,
            phase: "idle",
            version: None,
            downloaded: 0,
            total: None,
            ready_to_install: false,
            message: "检查更新".into(),
        }
    }
}
#[derive(Default)]
struct Inner {
    status: UpdateStatus,
    update: Option<Update>,
    verified: Option<Arc<Vec<u8>>>,
    control: Option<watch::Sender<bool>>,
    last_progress: Option<Instant>,
}
#[derive(Clone, Default)]
pub struct ApplicationUpdate(Arc<Mutex<Inner>>);
impl ApplicationUpdate {
    fn progress(&self, app: &AppHandle, length: usize, total: Option<u64>) {
        let status = {
            let mut inner = self.0.lock().unwrap_or_else(|error| error.into_inner());
            inner.status.downloaded += length as u64;
            inner.status.total = total;
            if inner
                .last_progress
                .is_some_and(|last| last.elapsed() < Duration::from_millis(80))
            {
                return;
            }
            inner.last_progress = Some(Instant::now());
            inner.status.revision += 1;
            inner.status.clone()
        };
        let _ = app.emit("application-update-status", status);
    }
    fn change(&self, app: &AppHandle, edit: impl FnOnce(&mut Inner)) -> UpdateStatus {
        let status = {
            let mut inner = self.0.lock().unwrap_or_else(|error| error.into_inner());
            edit(&mut inner);
            inner.status.revision += 1;
            inner.status.clone()
        };
        let _ = app.emit("application-update-status", &status);
        status
    }
    fn fail(&self, app: &AppHandle, code: &str, reason: impl std::fmt::Display) -> UpdateStatus {
        self.change(app, |inner| {
            inner.control = None;
            inner.status.phase = "failed";
            inner.status.message = format!("更新失败：{reason}（错误码：{code}）");
        })
    }
}

// Stop polling the same official future while paused. Its verified bytes are
// never reconstructed, and no custom downloader or signature verifier is used.
async fn pausable<T>(future: impl Future<Output = T>, mut paused: watch::Receiver<bool>) -> T {
    tokio::pin!(future);
    loop {
        if *paused.borrow_and_update() {
            if paused.changed().await.is_err() {
                return future.await;
            }
            continue;
        }
        tokio::select! { biased;
            change = paused.changed() => { if change.is_err() { return future.await; } }
            result = &mut future => return result,
        }
    }
}

fn safe_reason(error: &tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error;
    match error {
        Error::Reqwest(error) if error.is_timeout() => "网络请求超时".into(),
        Error::Reqwest(error) if error.is_connect() => "无法连接更新服务".into(),
        Error::Reqwest(error) => error
            .status()
            .map(|status| format!("HTTP {status}"))
            .unwrap_or_else(|| "更新连接中断，请重试".into()),
        Error::Minisign(_) | Error::Base64(_) | Error::SignatureUtf8(_) => {
            "更新签名校验失败，未安装".into()
        }
        Error::Network(message) if message.starts_with("Download request failed with status:") => {
            message.clone()
        }
        Error::ReleaseNotFound => "更新服务没有返回有效的版本信息".into(),
        Error::TargetNotFound(_) | Error::TargetsNotFound(_) => "没有当前平台的更新包".into(),
        Error::Io(error) => format!(
            "本地文件操作失败（{:?}，OS {:?}）",
            error.kind(),
            error.raw_os_error()
        ),
        Error::EmptyEndpoints => "更新源未配置（EmptyEndpoints）".into(),
        Error::Serialization(error) => format!(
            "更新清单格式错误（JSON 第 {} 行第 {} 列）",
            error.line(),
            error.column()
        ),
        Error::Semver(_) => "更新版本号格式错误（Semver）".into(),
        Error::UrlParse(_) | Error::InsecureTransportProtocol => {
            "更新地址无效或协议不安全（Url/Protocol）".into()
        }
        Error::AuthenticationFailed => "安装权限认证失败或已取消（AuthenticationFailed）".into(),
        Error::InvalidUpdaterFormat | Error::BinaryNotFoundInArchive => {
            "更新包不含有效的应用程序（Format/BinaryNotFound）".into()
        }
        Error::TempDirNotFound
        | Error::TempDirNotOnSameMountPoint
        | Error::FailedToDetermineExtractPath => {
            "无法准备更新暂存目录（TempDir/ExtractPath）".into()
        }
        _ => "更新器无法完成操作，请核对更新配置与安装权限".into(),
    }
}

#[tauri::command]
pub fn application_update_status(state: State<'_, ApplicationUpdate>) -> UpdateStatus {
    state
        .0
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .status
        .clone()
}

#[tauri::command]
pub async fn application_update_check(app: AppHandle) -> UpdateStatus {
    let state = app.state::<ApplicationUpdate>().inner().clone();
    {
        let mut inner = state.0.lock().unwrap_or_else(|error| error.into_inner());
        if matches!(
            inner.status.phase,
            "checking" | "downloading" | "paused" | "downloaded" | "installing"
        ) {
            return inner.status.clone();
        }
        inner.status.phase = "checking";
    }
    state.change(&app, |inner| {
        inner.update = None;
        inner.verified = None;
        inner.status = UpdateStatus {
            revision: inner.status.revision,
            phase: "checking",
            message: "正在检查更新…".into(),
            ..UpdateStatus::default()
        };
    });
    let result = match app
        .updater_builder()
        .timeout(Duration::from_secs(30))
        .build()
    {
        Ok(updater) => updater.check().await,
        Err(error) => Err(error),
    };
    match result {
        Ok(update) => state.change(&app, |inner| {
            inner.status.version = update.as_ref().map(|value| value.version.clone());
            inner.status.phase = if update.is_some() {
                "available"
            } else {
                "upToDate"
            };
            inner.status.message = update
                .as_ref()
                .map(|value| format!("下载更新 {}", value.version))
                .unwrap_or_else(|| "当前已是最新版本。".into());
            inner.update = update;
        }),
        Err(error) => state.fail(&app, "update_check_failed", safe_reason(&error)),
    }
}

#[tauri::command]
pub async fn application_update_download(app: AppHandle) -> UpdateStatus {
    let state = app.state::<ApplicationUpdate>().inner().clone();
    let (update, receiver) = {
        let mut inner = state.0.lock().unwrap_or_else(|error| error.into_inner());
        if !matches!(inner.status.phase, "available" | "failed") || inner.verified.is_some() {
            return inner.status.clone();
        }
        let Some(update) = inner.update.clone() else {
            return inner.status.clone();
        };
        let (sender, receiver) = watch::channel(false);
        inner.control = Some(sender);
        inner.status.phase = "downloading";
        (update, receiver)
    };
    state.change(&app, |inner| {
        inner.status.downloaded = 0;
        inner.status.total = None;
        inner.last_progress = None;
        inner.status.message = format!("正在下载 {}，点击暂停", update.version);
    });
    let progress_state = state.clone();
    let progress_app = app.clone();
    let download = update.download(
        move |length, total| {
            progress_state.progress(&progress_app, length, total);
        },
        || {},
    );
    match pausable(download, receiver).await {
        Ok(bytes) => state.change(&app, |inner| {
            inner.verified = Some(Arc::new(bytes));
            inner.control = None;
            inner.status.ready_to_install = true;
            inner.status.phase = "downloaded";
            inner.status.message = format!("{} 已通过验签，正在准备安装…", update.version);
        }),
        Err(error) => state.fail(&app, "update_download_failed", safe_reason(&error)),
    }
}

#[tauri::command]
pub fn application_update_pause(
    app: AppHandle,
    state: State<'_, ApplicationUpdate>,
    paused: bool,
) -> UpdateStatus {
    state.change(&app, |inner| {
        if matches!(inner.status.phase, "downloading" | "paused") {
            if let Some(control) = &inner.control {
                let _ = control.send(paused);
            }
            inner.status.phase = if paused { "paused" } else { "downloading" };
            inner.status.message = if paused {
                "下载已暂停，点击继续"
            } else {
                "正在下载，点击暂停"
            }
            .into();
        }
    })
}

#[tauri::command]
pub async fn application_update_install(app: AppHandle) -> UpdateStatus {
    let state = app.state::<ApplicationUpdate>().inner().clone();
    let (update, bytes) = {
        let mut inner = state.0.lock().unwrap_or_else(|error| error.into_inner());
        if !matches!(inner.status.phase, "downloaded" | "failed") {
            return inner.status.clone();
        }
        let (Some(update), Some(bytes)) = (inner.update.clone(), inner.verified.clone()) else {
            return inner.status.clone();
        };
        inner.status.phase = "installing";
        (update, bytes)
    };
    state.change(&app, |inner| {
        inner.status.phase = "installing";
        inner.status.message = "正在安装更新…".into();
    });
    match tauri::async_runtime::spawn_blocking(move || update.install(bytes.as_ref())).await {
        Ok(Ok(())) => {
            app.restart();
        }
        Ok(Err(error)) => state.fail(&app, "update_install_failed", safe_reason(&error)),
        Err(_) => state.fail(&app, "update_worker_failed", "安装工作线程未完成"),
    }
}

#[cfg(test)]
#[path = "application_update/tests.rs"]
mod tests;
