use super::*;

fn restore(
    db: &mut Connection,
    current: &Resource,
    operation: i64,
    direction: &str,
    request: &str,
) -> Result<Resource, Error> {
    crate::hbs_storage::test_sources::restore(
        db,
        HistoryDraft {
            resource_id: current.id.clone(),
            expected_revision: current.revision,
            operation_revision: operation,
            direction: direction.into(),
            request_id: request.into(),
        },
    )
}
#[test]
fn clear_is_reversible_without_reconstructing_the_old_review_baseline() {
    let mut db = database();
    let target = paper(&db);
    let generated = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &reading("pending"), "generate"),
    )
    .unwrap()
    .resource;
    let cleared = decision(&mut db, &generated, "clear", None, None);
    let restored = restore(&mut db, &cleared, cleared.revision, "undo", "undo-clear").unwrap();
    assert_eq!(restored.content, generated.content);
    assert_eq!(restored.review_baseline, generated.review_baseline);
    assert_eq!(restored.pending_field_ids, generated.pending_field_ids);
    let redone = restore(&mut db, &restored, cleared.revision, "redo", "redo-clear").unwrap();
    assert!(redone.content.is_none());
    assert!(redone.review_baseline.is_none());
    assert!(redone.pending_field_ids.is_empty());
}

#[test]
fn manual_edits_and_keep_replay_exact_baselines_without_losing_repair_drafts() {
    let mut db = database();
    let target = paper(&db);
    let generated = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &reading("original"), "generate"),
    )
    .unwrap()
    .resource;
    let clean = decision(&mut db, &generated, "keep", Some("all"), None);
    let mut input = draft(&target.id, &reading("edited"), "human");
    input.expected_revision = clean.revision;
    input.editor_kind = "human".into();
    let edited = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    let kept = decision(&mut db, &edited, "keep", Some("all"), None);
    let mut invalid = draft(&target.id, "{invalid", "raw-repair");
    invalid.expected_revision = kept.revision;
    crate::hbs_storage::test_sources::write(&mut db, invalid).unwrap();
    let pending = restore(&mut db, &kept, kept.revision, "undo", "undo-keep").unwrap();
    assert_eq!(pending.content, edited.content);
    assert_eq!(pending.review_baseline, edited.review_baseline);
    assert_eq!(pending.pending_field_ids, edited.pending_field_ids);
    let old = restore(&mut db, &pending, edited.revision, "undo", "undo-edit").unwrap();
    assert_eq!(old.content, clean.content);
    assert_eq!(old.review_baseline, clean.review_baseline);
    let redone = restore(&mut db, &old, edited.revision, "redo", "redo-edit").unwrap();
    assert_eq!(redone.content, edited.content);
    assert_eq!(redone.review_baseline, edited.review_baseline);
    let accepted = restore(&mut db, &redone, kept.revision, "redo", "redo-keep").unwrap();
    assert_eq!(accepted.content, kept.content);
    assert_eq!(accepted.review_baseline, kept.review_baseline);
    assert!(accepted.pending_field_ids.is_empty());
    assert_eq!(
        read_text(&db, &target.id, Some("test-conversation"), false)
            .unwrap()
            .text,
        "{invalid"
    );
    assert_eq!(
        restore(&mut db, &redone, kept.revision, "redo", "redo-keep")
            .unwrap()
            .revision,
        accepted.revision
    );
}
#[test]
fn stale_or_agent_history_cannot_overwrite_new_content_or_publish_a_template() {
    let mut db = database();
    let target = paper(&db);
    let generated = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &reading("original"), "generate"),
    )
    .unwrap()
    .resource;
    assert!(matches!(
        restore(
            &mut db,
            &generated,
            generated.revision,
            "undo",
            "undo-agent"
        ),
        Err(Error::Invalid(_))
    ));
    let kept = decision(&mut db, &generated, "keep", Some("all"), None);
    let mut input = draft(&target.id, &reading("another writer"), "outside");
    input.expected_revision = kept.revision;
    let changed = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    assert!(matches!(
        restore(&mut db, &kept, kept.revision, "undo", "stale-version"),
        Err(Error::Conflict)
    ));
    assert!(matches!(
        restore(&mut db, &changed, kept.revision, "undo", "stale-transition"),
        Err(Error::Conflict)
    ));
    assert_eq!(get(&db, &target.id).unwrap().content, changed.content);
    let template = ensure_paper_template(&db, "paper").unwrap();
    assert!(matches!(
        restore(&mut db, &template, 1, "undo", "template"),
        Err(Error::Invalid(_))
    ));
}
#[test]
fn first_human_write_can_restore_empty_identity_and_redo_the_saved_document() {
    let mut db = database();
    let target = paper(&db);
    let mut input = draft(&target.id, &reading("manual first body"), "human-first");
    input.editor_kind = "human".into();
    let written = crate::hbs_storage::test_sources::write(&mut db, input)
        .unwrap()
        .resource;
    let empty = restore(&mut db, &written, written.revision, "undo", "undo-first").unwrap();
    assert_eq!(empty.id, target.id);
    assert!(empty.content.is_none());
    let redone = restore(&mut db, &empty, written.revision, "redo", "redo-first").unwrap();
    assert_eq!(redone.content, written.content);
    assert_eq!(redone.review_baseline, written.review_baseline);
}
