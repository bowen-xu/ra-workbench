//! Original manual relation lifecycle through real Core and an independent literature package.
use super::*;
use research_workbench_native_sdk::knowledge_promotion::relation::{
    RelationAssertion, RelationAssertionDraft,
};
async fn fixture() -> Fixture {
    let f = evidence_matrix::fixture().await;
    seed(&f);
    f
}
fn seed(f: &Fixture) {
    f.store.connection().unwrap().execute_batch("insert into note_pages(id,title,position,status,is_favorite,revision,schema_version,created_at,updated_at) values('relation-note','Relation Ω note',0,'active',0,0,1,'now','now');
    insert into concepts(id,label,explanation,created_at,updated_at) values('relation-concept','Relation Ω concept','','now','now');
    insert into terms(id,text,normalized_text,language,created_at,updated_at) values('relation-term','Relation Ω concept','relation ω concept','en','now','now');
    insert into concept_terms(concept_id,term_id,relation_kind,created_at) values('relation-concept','relation-term','preferred','now');
    insert into definition_claims(id,concept_id,statement,scope_note,provenance_kind,epistemic_role,revision,created_at,updated_at) values('relation-definition','relation-concept','Relation Ω definition','scope','human_original','definition',1,'now','now');
    insert into structured_reading_units(id,project_id,paper_id,unit_kind,title,body,provenance_kind,epistemic_role,revision,created_at,updated_at) values('relation-reading','matrix-project','paper','research_question','Relation Ω reading','body','human_original','question',1,'now','now');").unwrap();
}
fn draft() -> RelationAssertionDraft {
    serde_json::from_value(json!({"projectId":"matrix-project","subject":{"kind":"concept","id":"relation-concept"},"predicateKind":"related_to","object":{"kind":"note_page","id":"relation-note"},"scopeNote":" Scope Ω ","anchorId":"matrix-first-anchor","evidenceStance":"supports","evidenceRationale":" Evidence Ω "})).unwrap()
}
async fn save(
    f: &Fixture,
    draft: RelationAssertionDraft,
) -> crate::runtime_plugins::native::NativeResult<RelationAssertion> {
    f.service
        .save_relation(f.store.clone(), f.key.clone(), draft)
        .await
}
fn counts(f: &Fixture) -> (i64, i64, i64) {
    f.store.connection().unwrap().query_row("select (select count(*) from relation_assertions),(select count(*) from relation_assertion_evidence_links),(select count(*) from research_object_revisions)",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap()
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_manual_relation_five_nodes_create_edit_history_and_reopen() {
    let f = fixture().await;
    let nodes = f
        .service
        .search_relation_nodes(f.store.clone(), f.key.clone(), "".into())
        .await
        .unwrap();
    for kind in [
        "concept",
        "definition_claim",
        "reading_unit",
        "paper",
        "note_page",
    ] {
        assert!(nodes.iter().any(|n| n.kind == kind), "{kind}");
    }
    let created = save(&f, draft()).await.unwrap();
    assert_eq!(created.scope_note, "Scope Ω");
    assert_eq!(created.evidence_count, 1);
    assert_eq!(created.subject.label, "Relation Ω concept");
    let mut edit = draft();
    edit.id = Some(created.id.clone());
    edit.expected_revision = Some(1);
    edit.scope_note = " Changed Ω ".into();
    edit.change_reason = Some(" exact reason ".into());
    f.store
        .connection()
        .unwrap()
        .execute(
            "update relation_assertions set provenance_kind='ai_accepted_after_edit' where id=?1",
            [&created.id],
        )
        .unwrap();
    let edited = save(&f, edit).await.unwrap();
    assert_eq!(edited.revision, 2);
    assert_eq!(edited.evidence_count, 1);
    assert_eq!(edited.provenance_kind, "ai_accepted_after_edit");
    let history = f
        .service
        .list_knowledge_revisions(
            f.store.clone(),
            f.key.clone(),
            "relation_assertion".into(),
            edited.id.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        history.iter().map(|r| r.revision).collect::<Vec<_>>(),
        vec![2, 1]
    );
    assert_eq!(history[0].change_reason, " exact reason ");
    assert_eq!(
        history[0].snapshot["subject"],
        json!({"kind":"concept","id":"relation-concept","label":"Relation Ω concept"})
    );
    assert_eq!(
        history[0].snapshot["provenanceKind"],
        "ai_accepted_after_edit"
    );
    f.store
        .connection()
        .unwrap()
        .execute(
            "update concepts set label='Renamed' where id='relation-concept'",
            [],
        )
        .unwrap();
    let listed = f
        .service
        .list_relations(
            f.store.clone(),
            f.key.clone(),
            Some("matrix-project".into()),
        )
        .await
        .unwrap();
    assert_eq!(listed[0].subject.label, "Relation Ω concept");
    assert!(
        f.service
            .list_relations(f.store.clone(), f.key.clone(), Some("missing".into()))
            .await
            .unwrap()
            .is_empty()
    );
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(counts(&f), (1, 1, 2));
    let after = f
        .service
        .list_relations(f.store.clone(), f.key.clone(), None)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(after).unwrap(),
        serde_json::to_value(listed).unwrap()
    );
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_manual_relation_original_errors_precede_begin_and_late_failure_rolls_back()
{
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let f = fixture().await;
    let begins = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&begins);
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(move |c: AuthContext<'_>| {
            if matches!(c.action, AuthAction::Transaction { .. }) {
                seen.fetch_add(1, Ordering::SeqCst);
            }
            Authorization::Allow
        }));
    let mut invalid = draft();
    invalid.predicate_kind = "invalid".into();
    invalid.subject.id = "missing".into();
    let error = save(&f, invalid).await.unwrap_err();
    assert_eq!(error.message, "关系断言参数无效");
    assert!(error.request_id.is_some());
    assert!(error.instance_id.is_some());
    let mut invalid = draft();
    invalid.anchor_id = None;
    assert_eq!(
        save(&f, invalid).await.unwrap_err().message,
        "关系证据必须同时提供锚点和立场"
    );
    let mut invalid = draft();
    invalid.subject.id = "missing".into();
    invalid.expected_revision = Some(1);
    assert_eq!(save(&f, invalid).await.unwrap_err().code, "not_found");
    let mut invalid = draft();
    invalid.expected_revision = Some(1);
    assert_eq!(
        save(&f, invalid).await.unwrap_err().message,
        "新建关系不能指定现有 revision"
    );
    let mut invalid = draft();
    invalid.subject.id = "界".repeat(67);
    assert_eq!(save(&f, invalid).await.unwrap_err().message, "关系端点无效");
    let mut invalid = draft();
    invalid.object = invalid.subject.clone();
    assert_eq!(
        save(&f, invalid).await.unwrap_err().message,
        "关系的起点和终点不能是同一对象"
    );
    assert_eq!(begins.load(Ordering::SeqCst), 0);
    assert_eq!(counts(&f), (0, 0, 0));
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let created = save(&f, draft()).await.unwrap();
    let before = counts(&f);
    let mut edit = draft();
    edit.id = Some(created.id.clone());
    assert_eq!(
        save(&f, edit.clone()).await.unwrap_err().message,
        "修改关系必须提供 expectedRevision"
    );
    edit.expected_revision = Some(2);
    let conflict = save(&f, edit).await.unwrap_err();
    assert_eq!(conflict.code, "revision_conflict");
    assert_eq!(conflict.current_revision, Some(1));
    for table in [
        "research_object_revisions",
        "relation_assertion_evidence_links",
    ] {
        f.store.connection().unwrap().execute_batch(&format!("create temp trigger relation_fail before insert on {table} begin select raise(abort,'synthetic relation failure'); end;")).unwrap();
        let error = save(&f, draft()).await.unwrap_err();
        assert_eq!(error.code, "storage_error");
        assert_eq!(
            error.relations.as_ref().unwrap().outcome,
            crate::hbs_storage::boundary::HbsOutcome::RolledBack
        );
        assert_eq!(counts(&f), before);
        f.store
            .connection()
            .unwrap()
            .execute_batch("drop trigger relation_fail")
            .unwrap();
    }
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_manual_relation_unknown_commit_fences_and_reopens() {
    let f = fixture().await;
    f.store.connection().unwrap().commit_hook(Some(|| true));
    let error = save(&f, draft()).await.unwrap_err();
    assert_eq!(error.code, "data_operation_outcome_unknown");
    assert!(f.store.connection().is_err());
    let f = reopen_actual_fixture(f, true).await;
    assert_eq!(counts(&f), (0, 0, 0));
    save(&f, draft()).await.unwrap();
    assert_eq!(counts(&f), (1, 1, 1));
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_manual_relation_committed_read_failure_retains_write() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    use std::sync::atomic::{AtomicBool, Ordering};
    let f = fixture().await;
    let committed = Arc::new(AtomicBool::new(false));
    {
        let db = f.store.connection().unwrap();
        let after = Arc::clone(&committed);
        db.commit_hook(Some(move || {
            after.store(true, Ordering::SeqCst);
            false
        }));
        let after = Arc::clone(&committed);
        db.authorizer(Some(move |c: AuthContext<'_>| {
            if after.load(Ordering::SeqCst)
                && matches!(
                    c.action,
                    AuthAction::Read {
                        table_name: "relation_assertions",
                        ..
                    }
                )
            {
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    }
    let error = save(&f, draft()).await.unwrap_err();
    assert_eq!(
        error.relations.as_ref().unwrap().outcome,
        crate::hbs_storage::boundary::HbsOutcome::Committed
    );
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    assert_eq!(counts(&f), (1, 1, 1));
    let f = reopen_actual_fixture(f, false).await;
    assert_eq!(
        f.service
            .list_relations(f.store.clone(), f.key.clone(), None)
            .await
            .unwrap()
            .len(),
        1
    );
    evidence_matrix::close(f).await;
}
#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_manual_relation_search_preserves_group_limits_and_literal_query() {
    let f = fixture().await;
    let long = f
        .service
        .search_relation_nodes(f.store.clone(), f.key.clone(), "界".repeat(501))
        .await
        .unwrap_err();
    assert_eq!(long.code, "invalid_input");
    assert!(long.request_id.is_some());
    let db = f.store.connection().unwrap();
    for i in 0..25 {
        db.execute("insert into concepts(id,label,explanation,created_at,updated_at) values(?1,?1,'','now','now')",[format!("many-{i:02}")]).unwrap();
        db.execute("insert into concept_terms(concept_id,term_id,relation_kind,created_at) values(?1,'relation-term','preferred','now')",[format!("many-{i:02}")]).unwrap();
    }
    for i in 0..20 {
        db.execute("insert into definition_claims(id,concept_id,statement,scope_note,provenance_kind,epistemic_role,revision,created_at,updated_at) values(?1,'relation-concept',?1,'','human_original','definition',1,'now','now')",[format!("many-def-{i:02}")]).unwrap();
        db.execute("insert into structured_reading_units(id,paper_id,unit_kind,title,body,provenance_kind,epistemic_role,revision,created_at,updated_at) values(?1,'paper','research_question',?1,'body','human_original','question',1,'now','now')",[format!("many-unit-{i:02}")]).unwrap();
        db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values(?1,'journalArticle',?1,'now','now')",[format!("many-paper-{i:02}")]).unwrap();
    }
    drop(db);
    let nodes = f
        .service
        .search_relation_nodes(f.store.clone(), f.key.clone(), "  ".into())
        .await
        .unwrap();
    assert_eq!(nodes.len(), 60);
    assert!(nodes[..20].iter().all(|n| n.kind == "concept"));
    assert!(nodes[20..35].iter().all(|n| n.kind == "definition_claim"));
    assert!(nodes[35..50].iter().all(|n| n.kind == "reading_unit"));
    assert!(nodes[50..].iter().all(|n| n.kind == "paper"));
    assert!(
        f.service
            .search_relation_nodes(f.store.clone(), f.key.clone(), "%_".into())
            .await
            .unwrap()
            .is_empty()
    );
    evidence_matrix::close(f).await;
}
