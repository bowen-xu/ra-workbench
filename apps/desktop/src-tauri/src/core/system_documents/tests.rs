use super::*;
use std::sync::Arc;
use uuid::Uuid;

#[test]
fn active_content_and_review_baseline_survive_reopen() {
    let root = tempfile::tempdir().unwrap();
    let before = read(root.path(), "structured-task").unwrap();
    let text = format!("{}\n用户新增说明。\n", before.content);
    let changed = write(
        root.path(),
        &before.id,
        &before.revision,
        &text,
        &before.baseline,
    )
    .unwrap();
    assert_eq!(prompt(root.path(), &before.id).unwrap(), text);
    assert_ne!(changed.content, changed.baseline);
    assert!(
        write(
            root.path(),
            &before.id,
            &before.revision,
            &text,
            &before.baseline
        )
        .is_err()
    );
    let reopened = read(root.path(), &before.id).unwrap();
    assert_eq!(reopened.revision, changed.revision);
    let kept = write(root.path(), &before.id, &reopened.revision, &text, &text).unwrap();
    assert_eq!(kept.content, kept.baseline);
    let second = format!("{}\n另一处修改。\n", text);
    let changed = write(
        root.path(),
        &before.id,
        &kept.revision,
        &second,
        &kept.baseline,
    )
    .unwrap();
    let undone = write(
        root.path(),
        &before.id,
        &changed.revision,
        &changed.baseline,
        &changed.baseline,
    )
    .unwrap();
    assert_eq!(undone.content, text);
}
#[test]
fn closed_catalog_and_template_variables_are_validated() {
    let root = tempfile::tempdir().unwrap();
    for doc in storage::list(root.path()).unwrap() {
        registry::validate(&doc.id, &doc.content).unwrap();
    }
    assert!(registry::id_from_path("source://system/../../secret").is_err());
    assert!(registry::id_from_path("source://system/unknown").is_err());
    assert!(read(root.path(), "../anything").is_err());
    assert!(registry::validate("structured-task", "missing variables").is_err());
    assert!(registry::validate("structured-reading-contract", "{{unknown}}").is_err());
    assert!(
        registry::validate(
            "generate-paper-portrait",
            "---\nname: replacement\ndescription: x\n---\nbody"
        )
        .is_err()
    );
}
#[test]
fn agent_paths_match_exact_registered_objects_without_old_or_custom_aliases() {
    for document in registry::DEFINITIONS {
        let path = registry::path(document.id).unwrap();
        assert!(path.starts_with("source://agents/"));
        assert_eq!(registry::id_from_path(&path).unwrap(), document.id);
    }
    assert_eq!(
        registry::id_from_path("source://agents/prompts/./%73tructured-task.md").unwrap(),
        "structured-task"
    );
    assert!(registry::id_from_path("source://agents/prompts%2Fstructured-task.md").is_err());
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../../tests/fixtures/source-addresses.json"
    ))
    .unwrap();
    for path in fixture["unregistered"].as_array().unwrap() {
        assert!(
            registry::id_from_path(path.as_str().unwrap()).is_err(),
            "{path}"
        );
    }
}
#[test]
fn skill_override_and_durable_write_receipt_use_exact_document() {
    let root = tempfile::tempdir().unwrap();
    let before = read(root.path(), "generate-paper-portrait").unwrap();
    let content = format!("{}\n新规则。\n", before.content);
    let saved = storage::commit(
        root.path(),
        &before.id,
        &before.revision,
        &content,
        &before.baseline,
        "request-1",
    )
    .unwrap();
    assert_eq!(
        storage::receipt(root.path(), "request-1")
            .unwrap()
            .unwrap()
            .revision,
        saved.revision
    );
    assert!(storage::receipt(root.path(), "unknown").unwrap().is_none());
    assert_eq!(
        skill_override(
            root.path(),
            ".agents/skills/generate-paper-portrait/SKILL.md"
        )
        .unwrap(),
        Some(content)
    );
    assert!(
        skill_override(root.path(), "../generate-paper-portrait/SKILL.md")
            .unwrap()
            .is_none()
    );
    assert!(
        storage::commit(
            root.path(),
            "structured-task",
            "bad",
            "bad",
            "bad",
            "request-1"
        )
        .is_err()
    );
}
#[tokio::test]
async fn approval_is_exact_denial_and_cancel_do_not_grant() {
    let root = tempfile::tempdir().unwrap();
    let doc = read(root.path(), "structured-task").unwrap();
    let approvals = Arc::new(approvals::Approvals::default());
    let conversation = Uuid::new_v4();
    let ask = {
        let approvals = approvals.clone();
        let doc = doc.clone();
        tokio::spawn(async move {
            approvals
                .ask(conversation, &doc, "new text", &Default::default())
                .await
        })
    };
    tokio::task::yield_now().await;
    let pending = approvals.pending(conversation).unwrap();
    assert_eq!(pending.len(), 1);
    let id = Uuid::parse_str(&pending[0].id).unwrap();
    assert!(approvals.decide(Uuid::new_v4(), id, true).is_err());
    approvals.decide(conversation, id, false).unwrap();
    let reason: serde_json::Value = serde_json::from_str(&ask.await.unwrap().unwrap_err()).unwrap();
    assert_eq!(reason["code"], "edit_declined");
    assert_eq!(reason["operationId"], id.to_string());
    assert!(approvals.decide(conversation, id, true).is_err());
    assert_eq!(read(root.path(), &doc.id).unwrap().revision, doc.revision);
    let ask = {
        let approvals = approvals.clone();
        tokio::spawn(async move {
            approvals
                .ask(conversation, &doc, "other text", &Default::default())
                .await
        })
    };
    tokio::task::yield_now().await;
    assert_eq!(approvals.pending(conversation).unwrap().len(), 1);
    ask.abort();
    let _ = ask.await;
    assert!(approvals.pending(conversation).unwrap().is_empty());
}
#[cfg(unix)]
#[test]
fn symlink_storage_and_lock_are_rejected() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    let doc = read(root.path(), "structured-task").unwrap();
    std::os::unix::fs::symlink(
        outside.path(),
        root.path().join("builtin-documents.v1.json"),
    )
    .unwrap();
    assert!(read(root.path(), &doc.id).is_err());
    std::fs::remove_file(root.path().join("builtin-documents.v1.json")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join(".builtin-documents.lock"))
        .unwrap();
    assert!(
        write(
            root.path(),
            &doc.id,
            &doc.revision,
            &doc.content,
            &doc.baseline
        )
        .is_err()
    );
}

#[test]
fn application_skill_discovery_and_resource_reads_use_active_overrides() {
    use crate::agent::skills::{application, bundled, files, resources};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    bundled::install_defaults(&root).unwrap();
    let original = application::discover(&root)
        .unwrap()
        .skills
        .into_iter()
        .find(|skill| skill.name == "edit-structured-resources")
        .unwrap();
    let before = read(&root, "edit-structured-resources").unwrap();
    let content = format!("{}\nRuntime custom rule.\n", before.content);
    write(
        &root,
        &before.id,
        &before.revision,
        &content,
        &before.baseline,
    )
    .unwrap();
    let current = application::discover(&root)
        .unwrap()
        .skills
        .into_iter()
        .find(|skill| skill.name == original.name)
        .unwrap();
    assert_ne!(current.identity.revision, original.identity.revision);
    assert!(files::load_selected(&root, &original, true, 131072, |_| Ok(())).is_err());
    let loaded = files::load_selected(&root, &current, true, 131072, |_| Ok(())).unwrap();
    assert!(loaded.body.contains("Runtime custom rule."));
    let reference = read(&root, "reading-reference").unwrap();
    let text = format!("{}\nRuntime reference rule.\n", reference.content);
    write(
        &root,
        &reference.id,
        &reference.revision,
        &text,
        &reference.baseline,
    )
    .unwrap();
    let request = resources::ResourceRequest {
        identity: current.identity.clone(),
        path: "references/reading.md".into(),
        expected_revision: None,
    };
    assert_eq!(
        resources::load_resource(&root, &current, true, &request, 131072, |_| Ok(()))
            .unwrap()
            .text,
        text
    );
}
