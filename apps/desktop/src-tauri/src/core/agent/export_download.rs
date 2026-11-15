//! Download destination and naming are native-owned; no frontend bytes or paths.
use super::export_commands::{ExportCommandError, ExportRequest};
use super::repository::SqliteConversationCatalog;
use research_agent_core::conversation::ConversationExport;
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Manager, State};
use uuid::Uuid;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadResponse {
    pub schema_version: u8,
    pub conversation_id: Uuid,
    pub watermark: u64,
    pub destination_path: String,
    pub byte_size: u64,
}

#[tauri::command]
pub async fn agent_download_conversation(
    app: AppHandle,
    catalog: State<'_, SqliteConversationCatalog>,
    request: ExportRequest,
) -> Result<DownloadResponse, ExportCommandError> {
    let id = request.identity()?;
    let directory = app
        .path()
        .download_dir()
        .map_err(|_| ExportCommandError::Unavailable)?;
    let snapshot = catalog.export_snapshot(id).await?;
    tokio::task::spawn_blocking(move || download(&directory, snapshot))
        .await
        .map_err(|_| ExportCommandError::Unavailable)?
}

fn filename_stem(title: &str) -> String {
    let clean: String = title
        .chars()
        .map(|c| {
            if c.is_control() || "\\/:*?\"<>|".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let mut stem = clean
        .trim()
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .to_owned();
    if stem.is_empty() {
        stem = "conversation".into();
    }
    let prefix = stem.split('.').next().unwrap_or("").to_ascii_lowercase();
    if matches!(prefix.as_str(), "con" | "prn" | "aux" | "nul")
        || ((prefix.starts_with("com") || prefix.starts_with("lpt"))
            && prefix.len() == 4
            && matches!(prefix.as_bytes()[3], b'1'..=b'9'))
    {
        stem.insert(0, '_');
    }
    while stem.len() > 200 {
        stem.pop();
    }
    stem
}

fn download(
    directory: &Path,
    snapshot: ConversationExport,
) -> Result<DownloadResponse, ExportCommandError> {
    use ExportCommandError::Unavailable;
    let directory = directory.canonicalize().map_err(|_| Unavailable)?;
    if !directory.is_dir() {
        return Err(Unavailable);
    }
    let stem = filename_stem(&snapshot.conversation.title);
    let mut bytes =
        serde_json::to_vec_pretty(&snapshot).map_err(|_| ExportCommandError::Corrupt)?;
    bytes.push(b'\n');
    for index in 0..10_000 {
        let filename = if index == 0 {
            format!("{stem}.json")
        } else {
            format!("{stem} ({index}).json")
        };
        let destination = directory.join(filename);
        // Validate representation before publication, avoiding a missing receipt
        // caused by lossy conversion after successful filesystem writes.
        let destination_path = destination.to_str().ok_or(Unavailable)?.to_owned();
        match crate::export_file::write_new_file(&destination, &bytes) {
            Ok(()) => {
                return Ok(DownloadResponse {
                    schema_version: 1,
                    conversation_id: snapshot.conversation.id,
                    watermark: snapshot.watermark,
                    destination_path,
                    byte_size: bytes.len() as u64,
                });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(Unavailable),
        }
    }
    Err(Unavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_agent_core::{
        conversation::{ConversationId, ConversationSummary},
        protocol::ConversationProjection,
    };
    fn snapshot(title: &str) -> ConversationExport {
        let id = ConversationId::from_uuid(Uuid::from_u128(1));
        ConversationExport::from_projection(
            ConversationSummary {
                internal: false,
                id,
                title: title.into(),
                archived: false,
                durable_count: 0,
                created_at: "2026-09-10T00:00:00.000Z".into(),
                updated_at: "2026-09-10T00:00:00.000Z".into(),
            },
            &ConversationProjection::new(id),
        )
        .unwrap()
    }
    #[test]
    fn names_are_portable_and_utf8_bounded() {
        for (title, expected) in [
            ("论文讨论", "论文讨论"),
            ("../A:B?", "-A-B-"),
            ("CON", "_CON"),
            ("lpt1.txt", "_lpt1.txt"),
            ("...", "conversation"),
        ] {
            assert_eq!(filename_stem(title), expected);
        }
        let long = filename_stem(&"文".repeat(180));
        assert!(long.len() <= 200);
        assert!(long.chars().all(|c| c == '文'));
    }
    #[test]
    fn repeated_downloads_preserve_existing_files_and_receipts_match_content() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Title.json"), b"original").unwrap();
        for index in 1..=2 {
            let result = download(root.path(), snapshot("Title")).unwrap();
            assert_eq!(
                Path::new(&result.destination_path).file_name().unwrap(),
                format!("Title ({index}).json").as_str()
            );
            let bytes = std::fs::read(&result.destination_path).unwrap();
            assert_eq!(bytes.len() as u64, result.byte_size);
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                value["conversation"]["id"],
                result.conversation_id.to_string()
            );
            assert_eq!(value["watermark"], result.watermark);
        }
        assert_eq!(
            std::fs::read(root.path().join("Title.json")).unwrap(),
            b"original"
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 3);
        assert!(download(&root.path().join("missing"), snapshot("Title")).is_err());
    }
}
