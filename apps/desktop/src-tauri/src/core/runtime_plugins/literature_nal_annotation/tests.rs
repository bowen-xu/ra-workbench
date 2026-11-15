use super::{
    execute::apply,
    outcome::{AnnotationCause, AnnotationOutcome, AnnotationPhase, AnnotationRejection},
    resources::AnnotationResources,
};
use research_workbench_native_sdk::{
    nal_annotation::NalAnnotation,
    nal_annotation_prepare::{
        NalAnnotationDecision, NalAnnotationLinkPreparation, NalAnnotationLinkTarget,
        NalAnnotationPreparation, NalAnnotationPrepareInput,
    },
};
use rusqlite::{
    functions::FunctionFlags,
    hooks::{AuthAction, AuthContext, Authorization},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[path = "tests/fixture.rs"]
mod fixture;
#[path = "tests/transactions.rs"]
mod transactions;
use fixture::{ANNOTATION, Fixture, ORIGINAL_PAGE, PAGE};

#[test]
fn create_uses_original_core_writer_assigned_identity_trimmed_240_title_and_body() {
    let fixture = Fixture::new();
    let normalized = "文".repeat(240);
    let call = fixture.call(&format!(" \t{normalized}\n "));
    let annotation = apply(
        &mut fixture.db.lock().unwrap(),
        &fixture.resources,
        &call.captured,
        &call.input,
        &call.plan,
    )
    .unwrap();
    assert_eq!(
        annotation,
        NalAnnotation {
            id: ANNOTATION.into(),
            target_kind: "concept".into(),
            target_id: "target".into(),
            note_page_id: PAGE.into()
        }
    );
    let db = fixture.db.lock().unwrap();
    let document = crate::notes::load_document_value(&db, PAGE).unwrap();
    assert_eq!(document.title.as_deref(), Some(normalized.as_str()));
    assert_eq!(document.blocks.len(), 1);
    assert!(!document.blocks[0].id.is_empty());
    assert_eq!(
        document.blocks[0].content,
        serde_json::json!({"kind":"inline","items":[]})
    );
    let link: (String, String, String) = db
        .query_row(
            "select id,concept_id,note_page_id from nal_annotations",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(link, (ANNOTATION.into(), "target".into(), PAGE.into()));
    assert_eq!(
        db.query_row("select count(*) from hbs_note_baselines", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn existing_trashed_note_and_241_to_300_titles_never_create_or_query_note_activity() {
    for length in [241, 300] {
        let fixture = Fixture::new();
        let existing = NalAnnotation {
            id: "existing-annotation".into(),
            target_kind: "concept".into(),
            target_id: "target".into(),
            note_page_id: ORIGINAL_PAGE.into(),
        };
        fixture.db.lock().unwrap().execute_batch(
            "insert into nal_annotations(id,concept_id,note_page_id) values('existing-annotation','target','original-core-note');
             update note_pages set status='trashed' where id='original-core-note';",
        ).unwrap();
        let before = fixture.snapshot();
        let mut call = fixture.call(&"文".repeat(length));
        call.plan.decision = NalAnnotationDecision::Existing {
            annotation: existing.clone(),
        };
        let reads = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&reads);
        let mut db = fixture.db.lock().unwrap();
        db.authorizer(Some(move |context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "note_pages",
                ..
            } => {
                observed.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            }
            _ => Authorization::Allow,
        }));
        let result = apply(
            &mut db,
            &fixture.resources,
            &call.captured,
            &call.input,
            &call.plan,
        );
        db.authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
        assert_eq!(result.unwrap(), existing);
        assert_eq!(reads.load(Ordering::SeqCst), 0);
        drop(db);
        assert_eq!(fixture.snapshot(), before);
    }
}

#[test]
fn create_title_over_core_limit_keeps_original_wrapper_and_rolls_back_every_write() {
    let fixture = Fixture::new();
    let title = "private-annotation-title".repeat(12);
    assert!(title.chars().count() > 240 && title.chars().count() <= 300);
    let before = fixture.snapshot();
    let call = fixture.call(&title);
    let error = apply(
        &mut fixture.db.lock().unwrap(),
        &fixture.resources,
        &call.captured,
        &call.input,
        &call.plan,
    )
    .unwrap_err();
    assert_eq!(error.outcome, AnnotationOutcome::RolledBack);
    assert_eq!(error.problem.phase, AnnotationPhase::CreateNote);
    assert_eq!(error.to_string(), "笔记创建失败，请重试");
    assert!(matches!(
        &error.problem.cause,
        AnnotationCause::NoteCreation {
            code: "invalid_input",
            safe_cause: "笔记标题过长",
            database: None
        }
    ));
    assert!(
        !serde_json::to_string(&error)
            .unwrap()
            .contains("private-annotation-title")
    );
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn note_inserted_bad_fact_fails_second_full_load_before_link_and_rolls_back() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let call = fixture.call("Synthetic note");
    let attempted = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&attempted);
    let mut db = fixture.db.lock().unwrap();
    db.create_scalar_function(
        "observe_annotation_core_insert",
        0,
        FunctionFlags::SQLITE_UTF8,
        move |_| {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(0)
        },
    )
    .unwrap();
    db.execute_batch(
        "create temp trigger annotation_second_load after insert on note_pages when new.id='assigned-annotation-page'
         begin
           select observe_annotation_core_insert();
           insert into nal_statements(id,subject,predicate,origin)
           values('after-note-invalid','{\"kind\":\"variable\",\"name\":\"x\"}',
                  '{\"kind\":\"concept\",\"id\":\"target\"}','human');
         end;",
    ).unwrap();
    let error = apply(
        &mut db,
        &fixture.resources,
        &call.captured,
        &call.input,
        &call.plan,
    )
    .unwrap_err();
    assert_eq!(attempted.load(Ordering::SeqCst), 1);
    assert_eq!(error.outcome, AnnotationOutcome::RolledBack);
    assert_eq!(error.problem.phase, AnnotationPhase::LinkLoad);
    assert_eq!(error.to_string(), "关系数据结构无效");
    drop(db);
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn actual_note_activity_is_read_after_second_load_and_inactive_creation_rolls_back() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let call = fixture.call("Synthetic note");
    let mut db = fixture.db.lock().unwrap();
    db.execute_batch(
        "create temp trigger annotation_created_trashed after insert on note_pages when new.id='assigned-annotation-page'
         begin update note_pages set status='trashed' where id=new.id;end;",
    ).unwrap();
    let error = apply(
        &mut db,
        &fixture.resources,
        &call.captured,
        &call.input,
        &call.plan,
    )
    .unwrap_err();
    assert_eq!(error.outcome, AnnotationOutcome::RolledBack);
    assert_eq!(error.problem.phase, AnnotationPhase::LinkNoteActive);
    assert!(matches!(
        &error.problem.cause,
        AnnotationCause::Rejected(AnnotationRejection::InactiveNote)
    ));
    assert_eq!(error.to_string(), "笔记不存在或不在活跃状态");
    drop(db);
    assert_eq!(fixture.snapshot(), before);
}
