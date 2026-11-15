//! Last-resort, human-selected recovery export. Does not open the application DB.
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryExportError {
    code: &'static str,
    message: String,
    os_error: Option<i32>,
}

fn write_recovery(destination: &Path, content: &str) -> Result<(), RecoveryExportError> {
    if !destination.is_absolute() || content.len() > 256 * 1024 * 1024 {
        return Err(RecoveryExportError {
            code: "NOTE_RECOVERY_EXPORT_INVALID",
            message: "导出路径必须为绝对路径，恢复文件不能超过 256 MiB；原数据未清理。".into(),
            os_error: None,
        });
    }
    crate::export_file::write_new_file(destination, content.as_bytes()).map_err(|error| {
        RecoveryExportError {
            code: "NOTE_RECOVERY_EXPORT_IO",
            message: format!("恢复文件写入失败（{:?}）；原数据未清理。", error.kind()),
            os_error: error.raw_os_error(),
        }
    })
}

#[tauri::command]
pub async fn export_note_recovery_file(
    destination: String,
    content: String,
) -> Result<(), RecoveryExportError> {
    tokio::task::spawn_blocking(move || write_recovery(Path::new(&destination), &content))
        .await
        .map_err(|_| RecoveryExportError {
            code: "NOTE_RECOVERY_EXPORT_WORKER",
            message: "恢复导出工作线程未完成；原数据未清理。".into(),
            os_error: None,
        })?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exports_without_database_and_never_overwrites_an_existing_file() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("recovery.json");
        write_recovery(&path, "{\"content\":\"synthetic\"}").unwrap();
        let first = std::fs::read(&path).unwrap();
        assert_eq!(
            write_recovery(&path, "{}").unwrap_err().code,
            "NOTE_RECOVERY_EXPORT_IO"
        );
        assert_eq!(std::fs::read(&path).unwrap(), first);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
        assert_eq!(
            write_recovery(Path::new("relative.json"), "{}")
                .unwrap_err()
                .code,
            "NOTE_RECOVERY_EXPORT_INVALID"
        );
    }
}
