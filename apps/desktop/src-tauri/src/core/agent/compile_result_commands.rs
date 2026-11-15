//! Identity-checked access to temporary results produced by the approved
//! writing workspace compiler. These files are deliberately separate from
//! formal manuscript compile runs and expire automatically.

use super::compile_synctex::{
    AgentCompileSynctex, AgentCompileSynctexQuery, SynctexPdfLocation, SynctexSourceLocation,
};
use crate::agent::writing_submission::{WritingSubmissionReceipt, WritingSubmissionService};
use crate::storage::AppStore;
use serde_json::Value;
use std::fs;
use tauri::{Manager, State};

#[path = "compile_result_commands/query.rs"]
mod query;
use uuid::Uuid;

const RESULT_TTL_SECONDS: u64 = 3600;

pub(crate) fn cleanup_expired_agent_compile_results(store: &AppStore) -> Result<(), String> {
    let parent = store
        .writing_cache_directory()
        .join("agent-compile-results");
    if !parent.exists() {
        return Ok(());
    }
    cleanup_expired_results(&parent).map_err(|_| "compile_result_cleanup_failed".to_owned())
}

fn cleanup_expired_results(parent: &std::path::Path) -> Result<(), std::io::Error> {
    let now = std::time::SystemTime::now();
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if Uuid::parse_str(name).is_err() || !path.is_dir() {
            continue;
        }
        let metadata_path = path.join("metadata.json");
        let metadata_bytes = match fs::read(&metadata_path) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let metadata = match serde_json::from_slice::<Value>(&metadata_bytes) {
            Ok(value) => value,
            Err(_) => continue,
        };
        let expired = result_expired(&metadata, &metadata_path, now).unwrap_or(false);
        if expired {
            fs::remove_dir_all(path)?;
        }
    }
    Ok(())
}

fn result_expired(
    metadata: &Value,
    metadata_path: &std::path::Path,
    now: std::time::SystemTime,
) -> Result<bool, String> {
    let created = match metadata["createdAtUnixSeconds"].as_u64() {
        Some(seconds) => std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_secs(seconds))
            .ok_or_else(|| "compile_result_expired".to_owned())?,
        None => fs::metadata(metadata_path)
            .and_then(|value| value.modified())
            .map_err(|_| "compile_result_expired".to_owned())?,
    };
    let age = now
        .duration_since(created)
        .map_err(|_| "compile_result_expired".to_owned())?;
    Ok(age.as_secs() >= RESULT_TTL_SECONDS)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_agent_compile_pdf(
    store: State<'_, AppStore>,
    result_id: String,
    conversation_id: String,
    request_id: String,
    turn_id: String,
    context_snapshot_id: String,
    manuscript_id: String,
    lease_id: String,
    manifest_hash: String,
) -> Result<Vec<u8>, String> {
    read_result_artifact(
        store.inner(),
        &result_id,
        "result.pdf",
        "pdf",
        &[
            ("conversationId", conversation_id),
            ("requestId", request_id),
            ("turnId", turn_id),
            ("contextSnapshotId", context_snapshot_id),
            ("manuscriptId", manuscript_id),
            ("leaseId", lease_id),
            ("manifestHash", manifest_hash),
        ],
    )
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) fn read_agent_compile_synctex(
    store: State<'_, AppStore>,
    result_id: String,
    conversation_id: String,
    request_id: String,
    turn_id: String,
    context_snapshot_id: String,
    manuscript_id: String,
    lease_id: String,
    manifest_hash: String,
) -> Result<Vec<u8>, String> {
    read_result_artifact(
        store.inner(),
        &result_id,
        "result.synctex.gz",
        "synctex",
        &[
            ("conversationId", conversation_id),
            ("requestId", request_id),
            ("turnId", turn_id),
            ("contextSnapshotId", context_snapshot_id),
            ("manuscriptId", manuscript_id),
            ("leaseId", lease_id),
            ("manifestHash", manifest_hash),
        ],
    )
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn forward_agent_compile_synctex(
    store: State<'_, AppStore>,
    app: tauri::AppHandle,
    result_id: String,
    conversation_id: String,
    request_id: String,
    turn_id: String,
    context_snapshot_id: String,
    manuscript_id: String,
    lease_id: String,
    manifest_hash: String,
    relative_path: String,
    line: u32,
    column: u32,
) -> Result<Vec<SynctexPdfLocation>, String> {
    forward_synctex_in(
        store.inner().clone(),
        app.try_state::<AgentCompileSynctex>()
            .map(|source| *source.inner()),
        result_id,
        conversation_id,
        request_id,
        turn_id,
        context_snapshot_id,
        manuscript_id,
        lease_id,
        manifest_hash,
        relative_path,
        line,
        column,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn forward_synctex_in(
    store: AppStore,
    synctex: impl Into<Option<AgentCompileSynctex>>,
    result_id: String,
    conversation_id: String,
    request_id: String,
    turn_id: String,
    context_snapshot_id: String,
    manuscript_id: String,
    lease_id: String,
    manifest_hash: String,
    relative_path: String,
    line: u32,
    column: u32,
) -> Result<Vec<SynctexPdfLocation>, String> {
    let synctex = synctex.into();
    let runtime = tokio::runtime::Handle::current();
    let identity = [
        ("conversationId", conversation_id.clone()),
        ("requestId", request_id.clone()),
        ("turnId", turn_id.clone()),
        ("contextSnapshotId", context_snapshot_id.clone()),
        ("manuscriptId", manuscript_id.clone()),
        ("leaseId", lease_id.clone()),
        ("manifestHash", manifest_hash.clone()),
    ];
    read_result_artifact(&store, &result_id, "result.pdf", "pdf", &identity)?;
    read_result_artifact(
        &store,
        &result_id,
        "result.synctex.gz",
        "synctex",
        &identity,
    )?;
    if line == 0 {
        return Err("invalid_path".to_owned());
    }
    let conversation_id =
        Uuid::parse_str(conversation_id.trim()).map_err(|_| "invalid_identity")?;
    let request_id = Uuid::parse_str(request_id.trim()).map_err(|_| "invalid_identity")?;
    let turn_id = Uuid::parse_str(turn_id.trim()).map_err(|_| "invalid_identity")?;
    let context_snapshot_id =
        Uuid::parse_str(context_snapshot_id.trim()).map_err(|_| "invalid_identity")?;
    let manuscript_id = Uuid::parse_str(manuscript_id.trim()).map_err(|_| "invalid_identity")?;
    let lease_id = Uuid::parse_str(lease_id.trim()).map_err(|_| "invalid_identity")?;
    let result_uuid = Uuid::parse_str(result_id.trim()).map_err(|_| "invalid_result_id")?;
    let manifest_hash = manifest_hash.trim().to_owned();
    let result_directory = store
        .writing_cache_directory()
        .join("agent-compile-results")
        .join(result_uuid.to_string());
    let pdf = result_directory.join("result.pdf");
    let prepared = WritingSubmissionService::from_app_store(&store)
        .restore_workspace(
            WritingSubmissionReceipt {
                conversation_id,
                request_id,
                manuscript_id,
            },
            context_snapshot_id,
        )
        .await;
    let snapshot = match prepared {
        Ok(prepared) => {
            let binding = &prepared.binding;
            if binding.conversation_id != conversation_id
                || binding.request_id != request_id
                || binding.turn_id != turn_id
                || binding.context_snapshot_id != context_snapshot_id
                || binding.manuscript_id != manuscript_id
                || binding.lease_id != lease_id
                || binding.manifest_hash != manifest_hash
                || prepared.lease.manifest().identity.lease_id != lease_id
            {
                return Err("compile_result_identity_mismatch".to_owned());
            }
            prepared
                .lease
                .working_directory()
                .map_err(|_| "compile_result_workspace_unavailable".to_owned())?
        }
        Err(_) => result_source_directory(&result_directory)?,
    };
    let relative_path = relative_path.trim().to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        // Preserve background ownership if the waiting command future is dropped.
        let query = query::Query::select(&store, synctex, runtime)?;
        let (toolchain_json, runtime_home) = query.prepare(&store)?;
        query.forward(
            &AgentCompileSynctexQuery {
                toolchain_json: &toolchain_json,
                runtime_home: &runtime_home,
                snapshot: &snapshot,
                output_directory: &result_directory,
                pdf: &pdf,
            },
            &relative_path,
            line,
            column,
        )
    })
    .await
    .map_err(|_| "synctex_unavailable".to_owned())?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn inverse_agent_compile_synctex(
    store: State<'_, AppStore>,
    app: tauri::AppHandle,
    result_id: String,
    conversation_id: String,
    request_id: String,
    turn_id: String,
    context_snapshot_id: String,
    manuscript_id: String,
    lease_id: String,
    manifest_hash: String,
    page: u32,
    x: f64,
    y: f64,
) -> Result<Vec<SynctexSourceLocation>, String> {
    inverse_synctex_in(
        store.inner().clone(),
        app.try_state::<AgentCompileSynctex>()
            .map(|source| *source.inner()),
        result_id,
        conversation_id,
        request_id,
        turn_id,
        context_snapshot_id,
        manuscript_id,
        lease_id,
        manifest_hash,
        page,
        x,
        y,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn inverse_synctex_in(
    store: AppStore,
    synctex: impl Into<Option<AgentCompileSynctex>>,
    result_id: String,
    conversation_id: String,
    request_id: String,
    turn_id: String,
    context_snapshot_id: String,
    manuscript_id: String,
    lease_id: String,
    manifest_hash: String,
    page: u32,
    x: f64,
    y: f64,
) -> Result<Vec<SynctexSourceLocation>, String> {
    let synctex = synctex.into();
    let runtime = tokio::runtime::Handle::current();
    let identity = [
        ("conversationId", conversation_id.clone()),
        ("requestId", request_id.clone()),
        ("turnId", turn_id.clone()),
        ("contextSnapshotId", context_snapshot_id.clone()),
        ("manuscriptId", manuscript_id.clone()),
        ("leaseId", lease_id.clone()),
        ("manifestHash", manifest_hash.clone()),
    ];
    read_result_artifact(&store, &result_id, "result.pdf", "pdf", &identity)?;
    read_result_artifact(
        &store,
        &result_id,
        "result.synctex.gz",
        "synctex",
        &identity,
    )?;
    if page == 0 || !x.is_finite() || !y.is_finite() || x < 0.0 || y < 0.0 {
        return Err("invalid_path".to_owned());
    }
    let conversation_id =
        Uuid::parse_str(conversation_id.trim()).map_err(|_| "invalid_identity")?;
    let request_id = Uuid::parse_str(request_id.trim()).map_err(|_| "invalid_identity")?;
    let turn_id = Uuid::parse_str(turn_id.trim()).map_err(|_| "invalid_identity")?;
    let context_snapshot_id =
        Uuid::parse_str(context_snapshot_id.trim()).map_err(|_| "invalid_identity")?;
    let manuscript_id = Uuid::parse_str(manuscript_id.trim()).map_err(|_| "invalid_identity")?;
    let lease_id = Uuid::parse_str(lease_id.trim()).map_err(|_| "invalid_identity")?;
    let result_uuid = Uuid::parse_str(result_id.trim()).map_err(|_| "invalid_result_id")?;
    let result_directory = store
        .writing_cache_directory()
        .join("agent-compile-results")
        .join(result_uuid.to_string());
    let pdf = result_directory.join("result.pdf");
    let prepared = WritingSubmissionService::from_app_store(&store)
        .restore_workspace(
            WritingSubmissionReceipt {
                conversation_id,
                request_id,
                manuscript_id,
            },
            context_snapshot_id,
        )
        .await;
    let snapshot = match prepared {
        Ok(prepared) => {
            if prepared.binding.conversation_id != conversation_id
                || prepared.binding.request_id != request_id
                || prepared.binding.turn_id != turn_id
                || prepared.binding.context_snapshot_id != context_snapshot_id
                || prepared.binding.manuscript_id != manuscript_id
                || prepared.binding.lease_id != lease_id
                || prepared.binding.manifest_hash != manifest_hash.trim()
                || prepared.lease.manifest().identity.lease_id != lease_id
            {
                return Err("compile_result_identity_mismatch".to_owned());
            }
            prepared
                .lease
                .working_directory()
                .map_err(|_| "compile_result_workspace_unavailable".to_owned())?
        }
        Err(_) => result_source_directory(&result_directory)?,
    };
    tauri::async_runtime::spawn_blocking(move || {
        // Preserve background ownership if the waiting command future is dropped.
        let query = query::Query::select(&store, synctex, runtime)?;
        let (toolchain_json, runtime_home) = query.prepare(&store)?;
        query.inverse(
            &AgentCompileSynctexQuery {
                toolchain_json: &toolchain_json,
                runtime_home: &runtime_home,
                snapshot: &snapshot,
                output_directory: &result_directory,
                pdf: &pdf,
            },
            page,
            x,
            y,
        )
    })
    .await
    .map_err(|_| "synctex_unavailable".to_owned())?
}

fn prepare_synctex_toolchain(
    store: &AppStore,
    synctex: AgentCompileSynctex,
) -> Result<(String, std::path::PathBuf), String> {
    let checked_at = {
        let connection = store
            .connection()
            .map_err(|_| "synctex_unavailable".to_owned())?;
        connection
            .query_row("select strftime('%Y-%m-%dT%H:%M:%fZ', 'now')", [], |row| {
                row.get(0)
            })
            .map_err(|_| "synctex_unavailable".to_owned())?
    };
    let runtime_home = store.writing_cache_directory().join("toolchain-home");
    let toolchain_json = (synctex.prepare_toolchain)(checked_at, &runtime_home)
        .map_err(|_| "synctex_unavailable".to_owned())?;
    Ok((toolchain_json, runtime_home))
}

fn result_source_directory(
    result_directory: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let source = result_directory.join("source");
    let metadata = fs::symlink_metadata(&source)
        .map_err(|_| "compile_result_workspace_unavailable".to_owned())?;
    if !metadata.file_type().is_dir() {
        return Err("compile_result_workspace_unavailable".to_owned());
    }
    let canonical_result = result_directory
        .canonicalize()
        .map_err(|_| "compile_result_workspace_unavailable".to_owned())?;
    let canonical_source = source
        .canonicalize()
        .map_err(|_| "compile_result_workspace_unavailable".to_owned())?;
    if canonical_source.parent() != Some(canonical_result.as_path()) {
        return Err("compile_result_workspace_unavailable".to_owned());
    }
    validate_source_snapshot(&canonical_source)?;
    Ok(canonical_source)
}

fn validate_source_snapshot(root: &std::path::Path) -> Result<(), String> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = 0usize;
    let mut bytes = 0u64;
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|_| "compile_result_workspace_unavailable".to_owned())?
        {
            let entry = entry.map_err(|_| "compile_result_workspace_unavailable".to_owned())?;
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|_| "compile_result_workspace_unavailable".to_owned())?;
            if metadata.file_type().is_symlink() {
                return Err("compile_result_workspace_unavailable".to_owned());
            }
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                files = files
                    .checked_add(1)
                    .ok_or_else(|| "compile_result_workspace_unavailable".to_owned())?;
                bytes = bytes
                    .checked_add(metadata.len())
                    .ok_or_else(|| "compile_result_workspace_unavailable".to_owned())?;
                if files > research_agent_core::workspace::MAX_FILES
                    || bytes > research_agent_core::workspace::MAX_WORKSPACE_BYTES
                {
                    return Err("compile_result_workspace_unavailable".to_owned());
                }
            } else {
                return Err("compile_result_workspace_unavailable".to_owned());
            }
        }
    }
    Ok(())
}

fn read_result_artifact(
    store: &AppStore,
    result_id: &str,
    file_name: &str,
    kind: &str,
    expected_identity: &[(&str, String)],
) -> Result<Vec<u8>, String> {
    read_result_artifact_at(
        store,
        result_id,
        file_name,
        kind,
        expected_identity,
        std::time::SystemTime::now(),
    )
}

fn read_result_artifact_at(
    store: &AppStore,
    result_id: &str,
    file_name: &str,
    kind: &str,
    expected_identity: &[(&str, String)],
    now: std::time::SystemTime,
) -> Result<Vec<u8>, String> {
    let result_id =
        Uuid::parse_str(result_id.trim()).map_err(|_| "invalid_result_id".to_owned())?;
    let base = store
        .writing_cache_directory()
        .join("agent-compile-results")
        .canonicalize()
        .map_err(|_| "compile_result_expired".to_owned())?;
    let directory = base.join(result_id.to_string());
    let canonical_directory = directory
        .canonicalize()
        .map_err(|_| "compile_result_expired".to_owned())?;
    if canonical_directory.parent() != Some(base.as_path()) {
        return Err("compile_result_invalid_path".to_owned());
    }
    let metadata_path = canonical_directory.join("metadata.json");
    let metadata = fs::read(&metadata_path).map_err(|_| "compile_result_expired".to_owned())?;
    let metadata: Value =
        serde_json::from_slice(&metadata).map_err(|_| "compile_result_corrupt".to_owned())?;
    if metadata["resultId"].as_str() != Some(result_id.to_string().as_str())
        || metadata["identity"].is_null()
        || metadata["identity"]["manifestHash"].as_str().is_none()
    {
        return Err("compile_result_identity_mismatch".to_owned());
    }
    for (key, expected) in expected_identity {
        if metadata["identity"][*key].as_str() != Some(expected.trim()) {
            return Err("compile_result_identity_mismatch".to_owned());
        }
    }
    if result_expired(&metadata, &metadata_path, now)? {
        return Err("compile_result_expired".to_owned());
    }
    let artifact_present = match kind {
        "pdf" => metadata["pdf"].as_bool() == Some(true),
        "synctex" => metadata["synctex"].as_bool() == Some(true),
        _ => false,
    };
    if !artifact_present {
        return Err("compile_result_kind_mismatch".to_owned());
    }
    let path = canonical_directory.join(file_name);
    let canonical_path = path
        .canonicalize()
        .map_err(|_| "compile_result_expired".to_owned())?;
    if canonical_path.parent() != Some(canonical_directory.as_path()) {
        return Err("compile_result_invalid_path".to_owned());
    }
    let bytes = fs::read(canonical_path).map_err(|_| "compile_result_expired".to_owned())?;
    if kind == "pdf" && !bytes.starts_with(b"%PDF-") {
        return Err("compile_result_invalid_pdf".to_owned());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{AppStore, migrate};
    use rusqlite::Connection;
    use tempfile::tempdir;

    #[test]
    fn rejects_identity_mismatch_before_reading_artifact() {
        let root = tempdir().unwrap();
        let store = AppStore::for_test(
            {
                let mut connection = Connection::open_in_memory().unwrap();
                migrate(&mut connection).unwrap();
                connection
            },
            root.path().join("objects"),
        );
        let result_id = Uuid::new_v4();
        let result = root
            .path()
            .join("agent-compile-results")
            .join(result_id.to_string());
        fs::create_dir_all(&result).unwrap();
        fs::write(result.join("result.pdf"), b"%PDF-test").unwrap();
        fs::write(
            result.join("metadata.json"),
            serde_json::json!({
                "resultId": result_id,
                "identity": {"manifestHash": "a"},
                "pdf": true,
            })
            .to_string(),
        )
        .unwrap();
        // Place the fixture at the same cache root used by AppStore.
        let expected = store
            .writing_cache_directory()
            .join("agent-compile-results")
            .join(result_id.to_string());
        fs::create_dir_all(expected.parent().unwrap()).unwrap();
        fs::rename(&result, &expected).unwrap();
        let error = read_result_artifact(
            &store,
            &result_id.to_string(),
            "result.pdf",
            "pdf",
            &[("manifestHash", "different".to_owned())],
        )
        .unwrap_err();
        assert_eq!(error, "compile_result_identity_mismatch");
    }

    #[test]
    fn rejects_expired_result_before_reading_artifact() {
        let root = tempdir().unwrap();
        let store = AppStore::for_test(
            {
                let mut connection = Connection::open_in_memory().unwrap();
                migrate(&mut connection).unwrap();
                connection
            },
            root.path().join("objects"),
        );
        let result_id = Uuid::new_v4();
        let result = root
            .path()
            .join("agent-compile-results")
            .join(result_id.to_string());
        fs::create_dir_all(&result).unwrap();
        fs::write(result.join("result.pdf"), b"%PDF-stale").unwrap();
        fs::write(
            result.join("metadata.json"),
            serde_json::json!({
                "resultId": result_id,
                "identity": {"manifestHash": "a"},
                "kind": "pdf",
            })
            .to_string(),
        )
        .unwrap();
        let expected = store
            .writing_cache_directory()
            .join("agent-compile-results")
            .join(result_id.to_string());
        fs::create_dir_all(expected.parent().unwrap()).unwrap();
        fs::rename(&result, &expected).unwrap();
        let now = std::time::SystemTime::now()
            .checked_add(std::time::Duration::from_secs(RESULT_TTL_SECONDS + 1))
            .unwrap();
        let error = read_result_artifact_at(
            &store,
            &result_id.to_string(),
            "result.pdf",
            "pdf",
            &[("manifestHash", "a".to_owned())],
            now,
        )
        .unwrap_err();
        assert_eq!(error, "compile_result_expired");
    }

    #[test]
    fn uses_persisted_creation_time_instead_of_metadata_mtime() {
        let root = tempdir().unwrap();
        let store = AppStore::for_test(
            {
                let mut connection = Connection::open_in_memory().unwrap();
                migrate(&mut connection).unwrap();
                connection
            },
            root.path().join("objects"),
        );
        let result_id = Uuid::new_v4();
        let expected = store
            .writing_cache_directory()
            .join("agent-compile-results")
            .join(result_id.to_string());
        fs::create_dir_all(&expected).unwrap();
        fs::write(expected.join("result.pdf"), b"%PDF-stale").unwrap();
        fs::write(
            expected.join("metadata.json"),
            serde_json::json!({
                "resultId": result_id,
                "identity": {"manifestHash": "a"},
                "createdAtUnixSeconds": 1,
                "pdf": true,
            })
            .to_string(),
        )
        .unwrap();

        let error = read_result_artifact(
            &store,
            &result_id.to_string(),
            "result.pdf",
            "pdf",
            &[("manifestHash", "a".to_owned())],
        )
        .unwrap_err();
        assert_eq!(error, "compile_result_expired");

        cleanup_expired_agent_compile_results(&store).unwrap();
        assert!(!expected.exists());
    }

    #[test]
    fn expires_at_the_exact_retention_boundary() {
        let root = tempdir().unwrap();
        let metadata_path = root.path().join("metadata.json");
        fs::write(&metadata_path, r#"{"createdAtUnixSeconds":100}"#).unwrap();
        let metadata: Value = serde_json::from_slice(&fs::read(&metadata_path).unwrap()).unwrap();
        let now = std::time::UNIX_EPOCH
            .checked_add(std::time::Duration::from_secs(100 + RESULT_TTL_SECONDS))
            .unwrap();
        assert!(result_expired(&metadata, &metadata_path, now).unwrap());
    }

    #[test]
    fn rejects_result_with_missing_artifact_kind() {
        let root = tempdir().unwrap();
        let store = AppStore::for_test(
            {
                let mut connection = Connection::open_in_memory().unwrap();
                migrate(&mut connection).unwrap();
                connection
            },
            root.path().join("objects"),
        );
        let result_id = Uuid::new_v4();
        let result = root
            .path()
            .join("agent-compile-results")
            .join(result_id.to_string());
        fs::create_dir_all(&result).unwrap();
        fs::write(result.join("result.pdf"), b"%PDF-missing-kind").unwrap();
        fs::write(
            result.join("metadata.json"),
            serde_json::json!({
                "resultId": result_id,
                "identity": {"manifestHash": "a"},
            })
            .to_string(),
        )
        .unwrap();
        let expected = store
            .writing_cache_directory()
            .join("agent-compile-results")
            .join(result_id.to_string());
        fs::create_dir_all(expected.parent().unwrap()).unwrap();
        fs::rename(&result, &expected).unwrap();
        let error = read_result_artifact(
            &store,
            &result_id.to_string(),
            "result.pdf",
            "pdf",
            &[("manifestHash", "a".to_owned())],
        )
        .unwrap_err();
        assert_eq!(error, "compile_result_kind_mismatch");
    }

    #[test]
    fn reads_artifacts_from_boolean_metadata_contract() {
        let root = tempdir().unwrap();
        let store = AppStore::for_test(
            {
                let mut connection = Connection::open_in_memory().unwrap();
                migrate(&mut connection).unwrap();
                connection
            },
            root.path().join("objects"),
        );
        let result_id = Uuid::new_v4();
        let expected = store
            .writing_cache_directory()
            .join("agent-compile-results")
            .join(result_id.to_string());
        fs::create_dir_all(&expected).unwrap();
        fs::write(expected.join("result.pdf"), b"%PDF-test").unwrap();
        fs::write(expected.join("result.synctex.gz"), b"synctex").unwrap();
        fs::write(
            expected.join("metadata.json"),
            serde_json::json!({
                "resultId": result_id,
                "identity": {"manifestHash": "a"},
                "pdf": true,
                "synctex": true,
            })
            .to_string(),
        )
        .unwrap();

        let pdf = read_result_artifact(
            &store,
            &result_id.to_string(),
            "result.pdf",
            "pdf",
            &[("manifestHash", "a".to_owned())],
        )
        .unwrap();
        let synctex = read_result_artifact(
            &store,
            &result_id.to_string(),
            "result.synctex.gz",
            "synctex",
            &[("manifestHash", "a".to_owned())],
        )
        .unwrap();
        assert_eq!(pdf, b"%PDF-test");
        assert_eq!(synctex, b"synctex");
    }

    #[test]
    fn accepts_identity_bound_source_snapshot_directory() {
        let root = tempdir().unwrap();
        let result = root.path().join(Uuid::new_v4().to_string());
        fs::create_dir_all(result.join("source")).unwrap();
        fs::write(result.join("source/main.tex"), b"snapshot").unwrap();
        let source = result_source_directory(&result).unwrap();
        assert_eq!(source, result.join("source").canonicalize().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_source_snapshot_escaping_result_directory() {
        let root = tempdir().unwrap();
        let result = root.path().join(Uuid::new_v4().to_string());
        fs::create_dir_all(&result).unwrap();
        std::os::unix::fs::symlink(root.path(), result.join("source")).unwrap();
        assert_eq!(
            result_source_directory(&result).unwrap_err(),
            "compile_result_workspace_unavailable"
        );
    }
}
