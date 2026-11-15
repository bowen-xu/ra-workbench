use super::*;

fn startup() -> StartupImportDraft {
    StartupImportDraft {
        catalog: None,
        presentations: vec![],
        contexts: vec![],
    }
}
fn manual(db: &Connection, paper: &str) {
    db.execute("insert into structured_reading_units(id,paper_id,unit_kind,title,body,epistemic_role,created_at,updated_at) values(?1,?2,'research_question','问题','原先的人工正文。','question','now','now')",rusqlite::params![format!("unit-{paper}"),paper]).unwrap();
}

#[test]
fn startup_imports_unopened_manual_papers_and_failed_work_can_be_retried_idempotently() {
    let mut db = database();
    let target = paper(&db);
    manual(&db, "paper");
    db.execute_batch("create trigger fail_startup_test before insert on structured_resource_legacy_versions when new.legacy_id='reading-unit:unit-paper' begin select raise(abort,'injected startup failure');end;").unwrap();
    let mut input = startup();
    input.catalog = Some(
        json!({"templates":[{"id":"old-template","label":"旧模板","fields":[{"id":"question","title":"问题","question":"所回应的问题。"}]}]}),
    );
    assert!(
        import_startup_with_evidence(
            Some(&crate::hbs_storage::test_sources::ReadingEvidence),
            &mut db,
            input.clone(),
            Some(crate::agent::writing_resources::test_sources::source())
        )
        .is_err()
    );
    assert!(!super::super::legacy::imported(&db, "paper:paper").unwrap());
    assert!(get(&db, &target.id).unwrap().content.is_none());
    db.execute_batch("drop trigger fail_startup_test;").unwrap();
    let result = import_startup_with_evidence(
        Some(&crate::hbs_storage::test_sources::ReadingEvidence),
        &mut db,
        input.clone(),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(result.imported_paper_count, 1);
    assert_eq!(result.imported_template_count, 0);
    let saved = get(&db, &target.id).unwrap();
    assert!(
        saved
            .content
            .as_ref()
            .unwrap()
            .to_string()
            .contains("原先的人工正文。")
    );
    // Already imported records are checked only by identity/marker on restart.
    db.execute(
        "update structured_resources set content_json='{}' where id=?1",
        [&target.id],
    )
    .unwrap();
    let repeated = import_startup_with_evidence(
        Some(&crate::hbs_storage::test_sources::ReadingEvidence),
        &mut db,
        input,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(repeated.imported_paper_count, 0);
    assert_eq!(get(&db, &target.id).unwrap().revision, saved.revision);
}

#[test]
fn legacy_write_and_adoption_guards_preserve_the_raw_draft_until_startup_finishes() {
    let mut db = database();
    let target = paper(&db);
    manual(&db, "paper");
    assert!(matches!(
        require_legacy_imported(&db, "paper"),
        Err(Error::LegacyImportRequired)
    ));
    let rejected = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &reading("尚未迁移时的写入。"), "too-early"),
    )
    .unwrap();
    assert_eq!(rejected.errors[0].code, "legacy_import_required");
    assert_eq!(get(&db, &target.id).unwrap().revision, 0);
    assert!(rejected.draft.is_some());
    let template = super::super::legacy::imported_template(
        &db,
        "global",
        "模板",
        &json!([{"id":"question","title":"问题","question":"问题？"}]),
    )
    .unwrap();
    assert!(matches!(
        crate::hbs_storage::test_sources::adopt(
            &mut db,
            AdoptionDraft {
                paper_id: "paper".into(),
                template_id: template.id,
                template_revision: template.revision,
                expected_revision: 0,
                confirmed: true,
                request_id: "too-early-adopt".into()
            }
        ),
        Err(Error::LegacyImportRequired)
    ));
    import_startup_with_evidence(
        Some(&crate::hbs_storage::test_sources::ReadingEvidence),
        &mut db,
        startup(),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    require_legacy_imported(&db, "paper").unwrap();
    assert!(
        read_text(&db, &target.id, Some("test-conversation"), false)
            .unwrap()
            .draft
            .is_some()
    );
    assert!(
        get(&db, &target.id)
            .unwrap()
            .content
            .unwrap()
            .to_string()
            .contains("原先的人工正文。")
    );
}

#[test]
fn late_legacy_import_reports_recovery_conflict_without_overwriting_or_marking_success() {
    let mut db = database();
    let mut target = paper(&db);
    manual(&db, "paper");
    target.content = Some(serde_json::from_str(&reading("已抢先写入的新正文。")).unwrap());
    target.review_baseline = target.content.clone();
    target.revision = 1;
    super::super::repository::persist_revision(&db, &target, "human", None, &[]).unwrap();
    assert!(matches!(
        import_startup_with_evidence(
            Some(&crate::hbs_storage::test_sources::ReadingEvidence),
            &mut db,
            startup(),
            Some(crate::agent::writing_resources::test_sources::source())
        ),
        Err(Error::LegacyImportConflict)
    ));
    assert!(!super::super::legacy::imported(&db, "paper:paper").unwrap());
    assert_eq!(get(&db, &target.id).unwrap().content, target.content);
    assert_eq!(
        db.query_row(
            "select body from structured_reading_units where id='unit-paper'",
            [],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "原先的人工正文。"
    );
}

#[test]
fn first_canonical_write_marks_empty_legacy_state_before_future_history_projections() {
    let mut db = database();
    let target = paper(&db);
    assert!(!super::super::legacy::imported(&db, "paper:paper").unwrap());
    let first = crate::hbs_storage::test_sources::write(
        &mut db,
        draft(&target.id, &reading("第一轮。"), "first-canonical"),
    )
    .unwrap()
    .resource;
    assert!(super::super::legacy::imported(&db, "paper:paper").unwrap());
    manual(&db, "paper");
    require_legacy_imported(&db, "paper").unwrap();
    let mut input = draft(&target.id, &reading("下一轮。"), "next-canonical");
    input.expected_revision = first.revision;
    assert_eq!(
        crate::hbs_storage::test_sources::write(&mut db, input)
            .unwrap()
            .status,
        "applied"
    );
}

#[test]
fn absent_domain_owners_keep_legacy_work_pending_while_core_startup_continues() {
    let mut db = database();
    let target = paper(&db);
    manual(&db, "paper");
    let core = uuid::Uuid::new_v4();
    let writing = uuid::Uuid::new_v4();
    let authoritative = uuid::Uuid::new_v4();
    let manuscript = uuid::Uuid::new_v4();
    let empty = json!({"schemaVersion":1,"persistent":[],"temporary":[]});
    for conversation in [core, writing, authoritative] {
        db.execute(
            "insert into agent_conversations(id,title) values(?1,'startup fixture')",
            [conversation.to_string()],
        )
        .unwrap();
    }
    db.execute("insert into agent_context_configurations(conversation_id,revision,state_json,updated_at) values(?1,1,?2,'now')",rusqlite::params![authoritative.to_string(),empty.to_string()]).unwrap();
    db.execute_batch("insert into research_projects(id,title,summary,created_at) values('startup-project','Synthetic','','now');").unwrap();
    db.execute("insert into manuscripts(id,project_id,title,created_at,updated_at) values(?1,'startup-project','Retained manuscript','now','now')", [manuscript.to_string()]).unwrap();
    db.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'main.tex','tex',0,?2,'now')",rusqlite::params![manuscript.to_string(),"a".repeat(64)]).unwrap();
    let writing_state = json!({"schemaVersion":1,"persistent":[{"label":"main.tex","resource":{"kind":"manuscriptFile","resourceId":manuscript,"path":"main.tex"}}],"temporary":[]});
    let mut input = startup();
    input.catalog = Some(
        json!({"templates":[{"id":"retained-template","label":"Retained template","fields":[{"id":"question","title":"Question","question":"What is known?"}]}]}),
    );
    input.contexts = vec![
        StartupContext {
            conversation_id: core,
            state: empty.clone(),
        },
        StartupContext {
            conversation_id: writing,
            state: writing_state.clone(),
        },
        StartupContext {
            conversation_id: authoritative,
            state: json!("corrupt cache must not replace native authority"),
        },
    ];
    for attempt in 0..2 {
        let pending = import_startup(None, &mut db, input.clone(), None).unwrap();
        assert!(pending.paper_ids.is_empty());
        assert_eq!(pending.deferred_paper_ids, ["paper"]);
        assert_eq!(pending.deferred_context_ids, [writing]);
        assert_eq!(pending.imported_paper_count, 0);
        assert_eq!(pending.imported_context_count, usize::from(attempt == 0));
        assert_eq!(
            pending.imported_template_count,
            if attempt == 0 { 4 } else { 0 }
        );
        assert!(!super::super::legacy::imported(&db, "paper:paper").unwrap());
        assert!(get(&db, &target.id).unwrap().content.is_none());
        assert!(
            crate::agent::context_session::configuration::read_in(&db, writing)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            crate::agent::context_session::configuration::read_in(&db, core)
                .unwrap()
                .unwrap()
                .state,
            empty
        );
    }
    let restored = import_startup_with_evidence(
        Some(&crate::hbs_storage::test_sources::ReadingEvidence),
        &mut db,
        input.clone(),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(restored.imported_paper_count, 1);
    assert_eq!(restored.imported_context_count, 1);
    assert!(restored.deferred_paper_ids.is_empty());
    assert!(restored.deferred_context_ids.is_empty());
    assert_eq!(
        crate::agent::context_session::configuration::read_in(&db, writing)
            .unwrap()
            .unwrap()
            .state,
        writing_state
    );
    assert!(
        get(&db, &target.id)
            .unwrap()
            .content
            .unwrap()
            .to_string()
            .contains("原先的人工正文。")
    );
    let repeated = import_startup_with_evidence(
        Some(&crate::hbs_storage::test_sources::ReadingEvidence),
        &mut db,
        input,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(
        (
            repeated.imported_paper_count,
            repeated.imported_context_count
        ),
        (0, 0)
    );
    assert!(
        !serde_json::to_value(repeated)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("deferredPaperIds")
    );
}

#[test]
fn fresh_core_startup_imports_real_templates_without_domain_sources() {
    let mut db = database();
    let imported = import_startup(None, &mut db, startup(), None).unwrap();
    assert_eq!(imported.imported_template_count, 3);
    assert_eq!(imported.imported_paper_count, 0);
    assert!(imported.deferred_paper_ids.is_empty());
    assert!(imported.deferred_context_ids.is_empty());
}
