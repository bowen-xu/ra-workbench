use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use fs2::FileExt;
use serde_json::Value;
use tokio::process::{Child, Command};

use super::{error::DoclingError, executable::prepare_executable, model::DoclingSettings};

pub(super) const STARTUP_TIMEOUT: Duration = Duration::from_secs(90);

pub(super) async fn acquire_start_lock(path: PathBuf) -> Result<File, DoclingError> {
    tokio::task::spawn_blocking(move || {
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)?;
        FileExt::lock_exclusive(&file)?;
        Ok::<_, std::io::Error>(file)
    })
    .await
    .map_err(|error| DoclingError::Start(format!("Docling 启动锁任务失败：{error}")))?
    .map_err(Into::into)
}

pub(super) fn spawn_daemon(
    settings: &DoclingSettings,
    daemon_script_path: &Path,
    registry_path: &Path,
    token_path: &Path,
    service_directory: &Path,
    model_directory: &Path,
    service_root: &Path,
) -> Result<Child, DoclingError> {
    let executable = prepare_executable(daemon_script_path, service_root)?;
    let mut command = Command::new(executable);
    command
        .arg("--model-directory")
        .arg(model_directory)
        .arg("--registry-path")
        .arg(registry_path)
        .arg("--token-path")
        .arg(token_path)
        .arg("--default-port")
        .arg(settings.default_port.to_string())
        .arg("--fallback-start-port")
        .arg(settings.fallback_start_port.to_string())
        .arg("--fallback-end-port")
        .arg(settings.fallback_end_port.to_string())
        .arg("--idle-shutdown-ms")
        .arg(settings.idle_shutdown_ms.to_string())
        .current_dir(service_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false);
    configure_model_environment(&mut command, model_directory);
    command
        .spawn()
        .map_err(|error| DoclingError::Start(format!("无法启动原生 Rust Docling：{error}")))
}

pub(super) fn configure_model_environment(command: &mut Command, model_directory: &Path) {
    // Remove all inherited engine/model overrides: only this verified FP32 bundle may load.
    for (key, _) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(|k| k.starts_with("DOCLING_") || k.starts_with("ORT_"))
        {
            command.env_remove(key);
        }
    }
    command
        .env("DOCLING_RS_FP32", "1")
        .env("DOCLING_RS_MODELS_DIR", model_directory)
        .env("DOCLING_RS_EP", "cpu")
        .env("DOCLING_RS_PDF_WORKERS", "2")
        .env(
            "DOCLING_LAYOUT_ONNX",
            model_directory.join("layout_heron.onnx"),
        )
        .env(
            "DOCLING_OCR_REC_ONNX",
            model_directory.join("ocr_rec_v6.onnx"),
        )
        .env(
            "DOCLING_OCR_DICT",
            model_directory.join("ocr_rec_v6_dict.txt"),
        )
        .env(
            "DOCLING_TABLEFORMER_ENCODER",
            model_directory.join("tableformer/encoder.onnx"),
        )
        .env(
            "DOCLING_TABLEFORMER_DECODER",
            model_directory.join("tableformer/decoder.onnx"),
        )
        .env(
            "DOCLING_TABLEFORMER_BBOX",
            model_directory.join("tableformer/bbox.onnx"),
        );
}

pub(super) async fn cleanup_failed_start(child: &mut Child) {
    let _ = child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
}

pub(super) fn recorded_pid_is_alive(path: &Path) -> bool {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|value| value.get("pid").and_then(Value::as_u64))
        .and_then(|pid| u32::try_from(pid).ok())
        .is_some_and(process_is_alive)
}

#[cfg(unix)]
pub(super) fn process_is_alive(pid: u32) -> bool {
    use nix::{errno::Errno, sys::signal::kill, unistd::Pid};

    match kill(Pid::from_raw(pid as i32), None) {
        Ok(()) | Err(Errno::EPERM) => true,
        Err(_) => false,
    }
}

#[cfg(not(unix))]
pub(super) fn process_is_alive(_pid: u32) -> bool {
    true
}

pub(super) fn create_private_directory(path: &Path) -> Result<(), DoclingError> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
