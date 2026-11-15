use super::*;
use research_workbench_native_sdk::duplicate_merge_prepare::{
    MergePreparationV1, MergeRejectionV1,
};
use rusqlite::hooks::{AuthAction, AuthContext, Authorization, TransactionOperation};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(crate) mod fixture;
mod legacy_inputs;
mod semantic;
mod transactions;
use fixture::Fixture;

#[test]
fn complete_merge_and_original_undo_preserve_the_core_note() {
    let f = Fixture::new();
    f.collection();
    f.reading_source();
    let before = f.data.note();
    let op = f.apply().unwrap();
    assert_eq!(op.state, "applied");
    assert_eq!(
        f.scalar::<i64>("select count(*) from merge_operations where state='applied'"),
        1
    );
    assert_eq!(
        f.scalar::<i64>("select count(*) from collection_memberships where item_id='survivor'"),
        1
    );
    assert_eq!(
        f.scalar::<i64>("select count(*) from nal_statements where subject like '%survivor%'"),
        1
    );
    assert_eq!(f.data.note(), before);
    assert_eq!(
        f.scalar::<String>(
            "select paper_id_snapshot from reading_note_sources where block_id='source-alias'"
        ),
        "survivor"
    );
    let package = f
        .catalog
        .discover()
        .unwrap()
        .packages
        .into_iter()
        .find(|p| p.package_key == f.key)
        .unwrap();
    let undo = crate::runtime_plugins::literature_duplicate_undo::UndoResources::load(
        &f.catalog, &package,
    )
    .unwrap();
    let captured = crate::runtime_plugins::literature_duplicate_undo::capture(
        &f.data.db.lock().unwrap(),
        &undo,
        op.id.clone(),
    )
    .unwrap();
    let plan = research_literature_library_native::duplicate_undo::prepare(
        &research_workbench_native_sdk::duplicate_undo::UndoMergePrepareInputV1 {
            snapshot_json: captured.snapshot_json.clone(),
        },
    )
    .unwrap();
    crate::runtime_plugins::literature_duplicate_undo::apply(
        &mut f.data.db.lock().unwrap(),
        &undo,
        &captured,
        &plan,
    )
    .unwrap();
    assert_eq!(
        f.scalar::<i64>(
            "select count(*) from bibliographic_items where id='source' and archived_at is null"
        ),
        1
    );
    assert_eq!(
        f.scalar::<i64>("select count(*) from collection_memberships where item_id='survivor'"),
        0
    );
    assert_eq!(f.data.note(), before);
    assert_eq!(
        f.scalar::<String>(
            "select paper_id_snapshot from reading_note_sources where block_id='source-alias'"
        ),
        "source"
    );
    assert_eq!(f.data.reopen().note(), before);
}

#[test]
fn missing_rejected_and_tampered_preparation_do_not_begin_a_transaction() {
    let f = Fixture::new();
    let captured = f.capture();
    let begins = Arc::new(AtomicUsize::new(0));
    let counter = begins.clone();
    f.data
        .db
        .lock()
        .unwrap()
        .authorizer(Some(move |ctx: AuthContext<'_>| {
            if matches!(
                ctx.action,
                AuthAction::Transaction {
                    operation: TransactionOperation::Begin
                }
            ) {
                counter.fetch_add(1, Ordering::SeqCst);
                Authorization::Deny
            } else {
                Authorization::Allow
            }
        }));
    assert_eq!(
        capture(
            &f.data.db.lock().unwrap(),
            &f.resources,
            "missing".into(),
            "survivor".into()
        )
        .err()
        .unwrap()
        .code(),
        "not_found"
    );
    let rejected = MergePreparationV1::Rejected {
        reason: MergeRejectionV1::NotFound,
    };
    assert_eq!(
        apply(
            &mut f.data.db.lock().unwrap(),
            &f.resources,
            &captured,
            &rejected
        )
        .unwrap_err()
        .problem
        .code(),
        "not_found"
    );
    for n in 0..7 {
        let mut plan = f.prepared(&captured);
        let MergePreparationV1::Ready {
            input,
            merged_item_id,
            operation_state,
            created_reading_state,
        } = &mut plan
        else {
            panic!()
        };
        match n {
            0 => input.candidate_id.push('x'),
            1 => input.survivor_item_id.push('x'),
            2 => input.left_item_id.push('x'),
            3 => input.right_item_id.push('x'),
            4 => merged_item_id.push('x'),
            5 => operation_state.push('x'),
            _ => *created_reading_state = true,
        }
        let e = apply(
            &mut f.data.db.lock().unwrap(),
            &f.resources,
            &captured,
            &plan,
        )
        .unwrap_err();
        assert_eq!(e.outcome, MergeOutcome::NotStarted);
        assert!(e.operation_id.is_none());
        assert_eq!(e.problem.code(), "plugin_host_response_mismatch");
    }
    assert_eq!(begins.load(Ordering::SeqCst), 0);
    f.data
        .db
        .lock()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
}

#[test]
fn preceding_outer_trigger_is_an_actual_legacy_input() {
    let f = Fixture::new();
    f.data.db.lock().unwrap().execute_batch("create trigger merge_add_legacy after update of archived_at on bibliographic_items
        when new.id='source' and old.archived_at is null and new.archived_at is not null begin
        insert into collections(id,name,description) values('late','Late collection','Late description');
        insert into collection_memberships(item_id,collection_id) values('source','late'); end;").unwrap();
    f.apply().unwrap();
    assert_eq!(
        f.scalar::<i64>("select count(*) from concepts where label='Late collection'"),
        1
    );
    assert_eq!(f.scalar::<i64>("select count(*) from collection_memberships where item_id='survivor' and collection_id='late'"),1);
}

#[test]
fn actual_trigger_metadata_is_snapshotted_without_prediction_rejection() {
    let f = Fixture::new();
    f.collection();
    f.data.db.lock().unwrap().execute_batch("create trigger merge_term_metadata after insert on terms when new.text='Original' begin
        update terms set revision=8,provenance_kind='ai_accepted_after_edit' where id=new.id; end;
        create trigger merge_concept_metadata after insert on concepts when new.label='Original' begin
        update concepts set revision=7,provenance_kind='ai_accepted_after_edit' where id=new.id;
        insert into terms(id,text,normalized_text,language,provenance_kind,revision,created_at,updated_at)
        values('actual-alias','Actual alias','actual alias','','human_original',1,'2026','2026');
        insert into concept_terms(concept_id,term_id,relation_kind,created_at) values(new.id,'actual-alias','alias','2026'); end;").unwrap();
    f.apply().unwrap();
    assert_eq!(f.scalar::<i64>("select count(*) from research_object_revisions where object_kind='term' and revision=1
        and json_extract(snapshot_json,'$.text')='Original' and json_extract(snapshot_json,'$.revision')=8"),1);
    assert_eq!(f.scalar::<i64>("select count(*) from research_object_revisions where object_kind='concept' and revision=1
        and json_extract(snapshot_json,'$.label')='Original' and json_extract(snapshot_json,'$.revision')=7"),1);
    assert_eq!(f.scalar::<i64>("select count(*) from semantic_representations where subject_id=(select id from concepts where label='Original') and subject_revision=7"),1);
    assert_eq!(f.scalar::<i64>("select count(*) from research_object_revisions where object_kind='concept'
        and json_extract(snapshot_json,'$.label')='Original' and json_array_length(snapshot_json,'$.terms')=2"),1);
}

#[test]
fn late_history_failure_rolls_back_every_original_write() {
    let f = Fixture::new();
    f.collection();
    f.reading_source();
    let note = f.data.note();
    let derived = f.derived();
    let before: Vec<(String, i64)> = [
        "concepts",
        "terms",
        "nal_statements",
        "research_object_revisions",
        "semantic_representations",
    ]
    .iter()
    .map(|t| {
        (
            t.to_string(),
            f.scalar(&format!("select count(*) from {t}")),
        )
    })
    .collect();
    f.data.db.lock().unwrap().execute_batch("create trigger merge_fail_history before insert on merge_operations begin select raise(abort,'synthetic history failure'); end;").unwrap();
    let e = f.apply().unwrap_err();
    assert_eq!(e.outcome, MergeOutcome::RolledBack);
    assert!(e.operation_id.is_some());
    for (t, count) in before {
        assert_eq!(f.scalar::<i64>(&format!("select count(*) from {t}")), count);
    }
    assert_eq!(
        f.scalar::<i64>("select count(*) from duplicate_candidates where decision is null"),
        1
    );
    assert_eq!(
        f.scalar::<i64>("select count(*) from bibliographic_items where archived_at is not null"),
        0
    );
    assert_eq!(f.data.note(), note);
    assert_eq!(f.derived(), derived);
    assert_eq!(f.data.reopen().note(), note);
}
