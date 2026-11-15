//! Bind returned source identities and counters to the actual Core read input.
use research_agent_core::submission::ContextSourceKind;
use research_workbench_native_sdk::{
    data::DataValue,
    writing_context::{ContextDecision, ContextInput},
};
use sha2::{Digest, Sha256};

pub(super) fn matches_context(input: &ContextInput, decision: &ContextDecision) -> bool {
    match (input, decision) {
        (_, ContextDecision::Rejected { .. })
        | (ContextInput::Validate { .. }, ContextDecision::Validated {})
        | (ContextInput::Project { .. }, ContextDecision::ProjectAccepted {})
        | (ContextInput::Registered { .. }, ContextDecision::Registered {}) => true,
        (
            ContextInput::CurrentFile {
                manuscript_id,
                relative_path,
                rows,
            },
            ContextDecision::CurrentFile { request },
        ) => {
            request.manuscript_id == *manuscript_id
                && matches!(request.files.as_slice(), [file]
                if file.relative_path == *relative_path && file.buffer.is_none() && file.selection.is_none()
                    && matches!(rows.rows.first().map(Vec::as_slice), Some([DataValue::Text(hash)]) if *hash == file.expected_disk_sha256))
        }
        (
            ContextInput::Materialize {
                manuscript_id,
                file,
                disk,
                disk_hash,
                used_bytes,
            },
            ContextDecision::Materialized {
                source,
                used_bytes: actual,
            },
        ) => {
            let full_text = file
                .buffer
                .as_ref()
                .map_or(disk.as_str(), |buffer| buffer.text.as_str());
            let anchor = source
                .anchor
                .as_ref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok());
            let expected = serde_json::json!({"manuscriptId":manuscript_id.to_string(), "diskSha256":disk_hash,
                "bufferRevision":file.buffer.as_ref().map(|buffer| buffer.revision),
                "startUtf16":file.selection.as_ref().map(|selection| selection.start_utf16),
                "endUtf16":file.selection.as_ref().map(|selection| selection.end_utf16)});
            source.kind == ContextSourceKind::Manuscript
                && source.resource_id == file.relative_path
                && source.revision == format!("sha256:{:x}", Sha256::digest(full_text))
                && source.source_id
                    == format!(
                        "writing:{:x}",
                        Sha256::digest(format!("{manuscript_id}/{}", file.relative_path))
                    )
                && source.text.len() <= full_text.len()
                && used_bytes.checked_add(disk.len().max(full_text.len())) == Some(*actual)
                && anchor.as_ref().is_some_and(|anchor| {
                    expected
                        .as_object()
                        .unwrap()
                        .iter()
                        .all(|(key, value)| anchor.get(key) == Some(value))
                })
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use research_workbench_native_sdk::{
        data::DataRows,
        writing_context::{WritingContextFile, WritingContextRequest},
    };

    #[test]
    fn materialized_source_cannot_replace_scope_basis_or_full_byte_counter() {
        use research_agent_core::submission::ContextText;
        let id = uuid::Uuid::nil();
        let source = ContextText {
            kind: ContextSourceKind::Manuscript,
            source_id: format!("writing:{:x}", Sha256::digest(format!("{id}/main.tex"))),
            resource_id: "main.tex".into(), revision: format!("sha256:{:x}", Sha256::digest("disk")),
            text: "disk".into(), anchor: Some(serde_json::json!({"manuscriptId":id.to_string(),"diskSha256":"a".repeat(64),"bufferRevision":null,"startUtf16":null,"endUtf16":null}).to_string()),
        };
        let input = ContextInput::Materialize {
            manuscript_id: id,
            file: WritingContextFile {
                relative_path: "main.tex".into(),
                expected_disk_sha256: "a".repeat(64),
                buffer: None,
                selection: None,
            },
            disk: "disk".into(),
            disk_hash: "a".repeat(64),
            used_bytes: 3,
        };
        assert!(matches_context(
            &input,
            &ContextDecision::Materialized {
                source: source.clone(),
                used_bytes: 7
            }
        ));
        assert!(!matches_context(
            &input,
            &ContextDecision::Materialized {
                source: source.clone(),
                used_bytes: 4
            }
        ));
        let mut changed = source.clone();
        changed.resource_id = "other.tex".into();
        assert!(!matches_context(
            &input,
            &ContextDecision::Materialized {
                source: changed,
                used_bytes: 7
            }
        ));
        let mut changed = source.clone();
        changed.anchor = Some("{}".into());
        assert!(!matches_context(
            &input,
            &ContextDecision::Materialized {
                source: changed,
                used_bytes: 7
            }
        ));
        let mut changed = source;
        changed.revision = "sha256:another".into();
        assert!(!matches_context(
            &input,
            &ContextDecision::Materialized {
                source: changed,
                used_bytes: 7
            }
        ));
    }
    #[test]
    fn current_file_cannot_replace_the_actual_manuscript_path_or_disk_basis() {
        let manuscript_id = uuid::Uuid::nil();
        let input = ContextInput::CurrentFile {
            manuscript_id,
            relative_path: "main.tex".into(),
            rows: DataRows {
                rows: vec![vec![DataValue::Text("a".repeat(64))]],
            },
        };
        let request = WritingContextRequest {
            manuscript_id,
            files: vec![WritingContextFile {
                relative_path: "main.tex".into(),
                expected_disk_sha256: "a".repeat(64),
                buffer: None,
                selection: None,
            }],
        };
        assert!(matches_context(
            &input,
            &ContextDecision::CurrentFile {
                request: request.clone()
            }
        ));
        let mut changed = request.clone();
        changed.files[0].relative_path = "other.tex".into();
        assert!(!matches_context(
            &input,
            &ContextDecision::CurrentFile { request: changed }
        ));
        let mut changed = request.clone();
        changed.manuscript_id = uuid::Uuid::from_u128(1);
        assert!(!matches_context(
            &input,
            &ContextDecision::CurrentFile { request: changed }
        ));
        let mut changed = request;
        changed.files[0].expected_disk_sha256 = "b".repeat(64);
        assert!(!matches_context(
            &input,
            &ContextDecision::CurrentFile { request: changed }
        ));
    }
}
