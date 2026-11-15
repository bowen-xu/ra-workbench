//! Retain identity-bound compile artifacts and remove ephemeral output.
use super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn finish_output(
    output_directory: PathBuf,
    root_path: String,
    outcome: &crate::agent::process::ProcessOutcome,
    result_id: Uuid,
    store: AppStore,
    receipt: WritingSubmissionReceipt,
    context_snapshot_id: Uuid,
    scope: &WorkspaceIdentity,
    manifest_hash: &str,
    lease: &crate::agent::workspace::WorkspaceLease,
    diagnostics: compiler::PreparedDiagnostics,
    raw_output: String,
) -> Result<ToolOutput, RunError> {
    let stem = Path::new(&root_path)
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or(RunError::ToolRejected)?;
    let pdf = artifact(&output_directory.join(format!("{stem}.pdf")))?;
    let synctex = artifact(&output_directory.join(format!("{stem}.synctex.gz")))?;
    let status = match outcome.reason {
        StopReason::Completed if outcome.exit.as_ref().and_then(|exit| exit.code) == Some(0) => {
            "succeeded"
        }
        StopReason::Completed => "failed",
        StopReason::Cancelled => "cancelled",
        StopReason::Deadline => "timed_out",
        StopReason::IoFailure => "io_failure",
    };
    let identity = json!({
        "conversationId": receipt.conversation_id,
        "requestId": receipt.request_id,
        "turnId": scope.turn_id,
        "contextSnapshotId": context_snapshot_id,
        "manuscriptId": receipt.manuscript_id,
        "leaseId": scope.lease_id,
        "manifestHash": manifest_hash,
    });
    let retained = persist_result(
        &store,
        CompileResultPersistence {
            result_id,
            root_path: &root_path,
            identity: &identity,
            output_directory: &output_directory,
            pdf: pdf.as_ref(),
            synctex: synctex.as_ref(),
            lease,
        },
    );
    let retained = retained?;
    let created_at_unix_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| RunError::EventSink)?
        .as_secs();
    let result = ToolOutput {
        content: json!({
            "status": status,
            "resultId": result_id,
            "compileVersion": result_id,
            "identity": identity,
            "rootPath": root_path,
            "exitCode": outcome.exit.as_ref().and_then(|exit| exit.code),
            "diagnostics": diagnostics.resolve(&raw_output),
            "rawOutput": tail(&raw_output, OUTPUT_LIMIT),
            "artifacts": {
                "pdf": pdf,
                "synctex": synctex,
            },
            "retained": retained,
            "createdAtUnixSeconds": created_at_unix_seconds,
            "expiresAfterSeconds": 3600,
        })
        .to_string(),
        is_error: status != "succeeded",
    };
    Ok(result)
}

struct CompileResultPersistence<'a> {
    result_id: Uuid,
    root_path: &'a str,
    identity: &'a serde_json::Value,
    output_directory: &'a Path,
    pdf: Option<&'a serde_json::Value>,
    synctex: Option<&'a serde_json::Value>,
    lease: &'a crate::agent::workspace::WorkspaceLease,
}

fn persist_result(store: &AppStore, input: CompileResultPersistence<'_>) -> Result<bool, RunError> {
    if input.pdf.is_none() && input.synctex.is_none() {
        return Ok(false);
    }
    let parent = store
        .writing_cache_directory()
        .join("agent-compile-results");
    fs::create_dir_all(&parent).map_err(|_| RunError::EventSink)?;
    crate::agent::compile_result_commands::cleanup_expired_agent_compile_results(store)
        .map_err(|_| RunError::EventSink)?;
    let directory = parent.join(input.result_id.to_string());
    fs::create_dir(&directory).map_err(|_| RunError::EventSink)?;
    let write_result = (|| -> Result<(), RunError> {
        if input.pdf.is_some() {
            fs::copy(
                input
                    .output_directory
                    .join(format_stem(input.root_path, ".pdf")?),
                directory.join("result.pdf"),
            )
            .map_err(|_| RunError::EventSink)?;
        }
        if input.synctex.is_some() {
            fs::copy(
                input
                    .output_directory
                    .join(format_stem(input.root_path, ".synctex.gz")?),
                directory.join("result.synctex.gz"),
            )
            .map_err(|_| RunError::EventSink)?;
        }
        let output_name = input
            .output_directory
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(RunError::EventSink)?;
        persist_source_snapshot(input.lease, &directory, output_name)?;
        let metadata = json!({
            "resultId": input.result_id,
            "rootPath": input.root_path,
            "identity": input.identity,
            "createdAtUnixSeconds": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| RunError::EventSink)?
                .as_secs(),
            "pdf": input.pdf.is_some(),
            "synctex": input.synctex.is_some(),
            "sourceSnapshot": true,
        });
        fs::write(directory.join("metadata.json"), metadata.to_string())
            .map_err(|_| RunError::EventSink)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_dir_all(&directory);
    }
    write_result.map(|()| true)
}

fn format_stem(root_path: &str, suffix: &str) -> Result<String, RunError> {
    let stem = Path::new(root_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or(RunError::ToolRejected)?;
    Ok(format!("{stem}{suffix}"))
}

fn persist_source_snapshot(
    lease: &crate::agent::workspace::WorkspaceLease,
    result_directory: &Path,
    excluded_directory: &str,
) -> Result<(), RunError> {
    let source = result_directory.join("source");
    fs::create_dir(&source).map_err(|_| RunError::EventSink)?;
    lease
        .visit_work_bytes(|file, bytes| {
            research_agent_core::workspace::validate_path(&file.path)
                .map_err(|_| crate::agent::workspace::LeaseError::Invalid)?;
            if file.path == excluded_directory
                || file
                    .path
                    .strip_prefix(excluded_directory)
                    .is_some_and(|rest| rest.starts_with('/'))
            {
                return Ok(());
            }
            let target = source.join(&file.path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .map_err(|_| crate::agent::workspace::LeaseError::Unavailable)?;
            }
            if fs::symlink_metadata(&target).is_ok() {
                return Err(crate::agent::workspace::LeaseError::Conflict);
            }
            fs::write(&target, bytes)
                .map_err(|_| crate::agent::workspace::LeaseError::Unavailable)?;
            Ok(())
        })
        .map_err(|_| RunError::EventSink)
}

fn artifact(path: &Path) -> Result<Option<serde_json::Value>, RunError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(RunError::EventSink),
    };
    if !metadata.file_type().is_file() || metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(RunError::EventSink);
    }
    if path.extension().and_then(|extension| extension.to_str()) == Some("pdf") {
        let mut header = [0_u8; 5];
        let mut file = fs::File::open(path).map_err(|_| RunError::EventSink)?;
        use std::io::Read;
        if file.read_exact(&mut header).is_err() || &header != b"%PDF-" {
            return Err(RunError::EventSink);
        }
    }
    Ok(Some(json!({"byteSize": metadata.len()})))
}

pub(super) fn remove_ephemeral_directory(path: &Path) -> Result<(), RunError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| RunError::EventSink)?;
    let canonical = path.canonicalize().map_err(|_| RunError::EventSink)?;
    if !metadata.file_type().is_dir() || canonical != path || path.parent().is_none() {
        return Err(RunError::EventSink);
    }
    fs::remove_dir_all(path).map_err(|_| RunError::EventSink)
}

pub(super) fn render_output(
    stdout: &crate::agent::process::RetainedOutput,
    stderr: &crate::agent::process::RetainedOutput,
) -> String {
    fn render(value: &crate::agent::process::RetainedOutput) -> String {
        let mut output = String::from_utf8_lossy(&value.head).into_owned();
        if value.omitted_bytes > 0 {
            output.push_str(&format!("\n[{} bytes omitted]\n", value.omitted_bytes));
        }
        output.push_str(&String::from_utf8_lossy(&value.tail));
        output
    }
    format!("{}\n{}", render(stdout), render(stderr))
}

pub(super) fn tail(value: &str, maximum_characters: usize) -> String {
    let count = value.chars().count();
    if count <= maximum_characters {
        return value.to_owned();
    }
    let start = value
        .char_indices()
        .nth(count - maximum_characters)
        .map_or(value.len(), |(index, _)| index);
    format!("…{}", &value[start..])
}
