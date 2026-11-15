use super::*;
use crate::agent::{
    composed_tools::RejectingToolExecutor,
    context_session::{
        resource_grants,
        sources::{Access, Part, Resource},
    },
    repository::submissions,
};
use research_agent_core::{protocol::ModelIdentity, submission::SubmissionInput};
use rusqlite::{Connection, params};
#[path = "tests/addresses.rs"]
mod addresses;
#[path = "tests/admission.rs"]
mod admission;
#[path = "tests/assets.rs"]
mod assets;
#[path = "tests/composition.rs"]
mod composition;
#[path = "tests/invariants.rs"]
mod invariants;
#[path = "tests/local_recovery.rs"]
mod local_recovery;
#[path = "tests/note_delivery.rs"]
mod note_delivery;
#[path = "tests/outcomes.rs"]
mod outcomes;
#[path = "tests/page_creation.rs"]
mod page_creation;
#[path = "tests/patch_previews.rs"]
mod patch_previews;
#[path = "tests/read_ranges.rs"]
mod read_ranges;
#[path = "tests/requests.rs"]
mod requests;
#[path = "tests/rich_review.rs"]
mod rich_review;
#[path = "tests/source_links.rs"]
mod source_links;
#[path = "tests/structured.rs"]
mod structured;
#[path = "tests/structured_freeze.rs"]
mod structured_freeze;
#[path = "tests/system_documents.rs"]
mod system_documents;
struct Fixture {
    _root: tempfile::TempDir,
    tools: TextTools,
    page: Uuid,
    parent: Uuid,
}
fn page(db: &Connection, title: &str, parent: Option<Uuid>, body: &str) -> Uuid {
    let id = Uuid::new_v4();
    db.execute("insert into note_pages(id,title,parent_page_id,position,status,is_favorite,revision,schema_version,created_at,updated_at) values(?1,?2,?3,0,'active',0,0,1,'now','now')",params![id.to_string(),title,parent.map(|id|id.to_string())]).unwrap();
    let document = crate::notes::NoteDocument {
        title: None,
        schema_version: 1,
        page_id: id.to_string(),
        blocks: vec![crate::notes::NoteDocumentBlock {
            id: Uuid::new_v4().to_string(),
            block_type: "paragraph".into(),
            props: json!({"privateSidecar":"kept"}),
            content: json!({"kind":"inline","items":[{"type":"text","text":body,"styles":[]}]}),
            children: vec![],
        }],
    };
    // Domain validation/history are the same as the ordinary save path.
    let tx = db.unchecked_transaction().unwrap();
    crate::notes::save_document_in(
        &tx,
        crate::notes::NoteDocumentSaveRequest {
            document,
            expected_revision: 0,
            save_reason: "manual".into(),
        },
    )
    .unwrap();
    tx.commit().unwrap();
    id
}
fn fixture(write: bool) -> Fixture {
    fixture_with_store(write, |root| {
        let mut db = Connection::open(root.join("fixture.sqlite3")).unwrap();
        crate::storage::migrate(&mut db).unwrap();
        AppStore::for_test(db, root.join("objects"))
    })
}
fn fixture_with_store(write: bool, open: impl FnOnce(&std::path::Path) -> AppStore) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let store = open(root.path());
    let mut db = store.connection().unwrap();
    let parent = page(&db, "Parent", None, "parent");
    let page = page(&db, "Note", None, "old");
    let conversation = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Text patch fixture')",
        [conversation.to_string()],
    )
    .unwrap();
    let resources = [parent, page].map(|id| Resource::NoteGroup {
        resource_id: id.to_string(),
        new_member_access: None,
        group_access: None,
        parts: vec![Part {
            resource: Resource::Note { resource_id: id },
            label: "Note".into(),
            selected: true,
            access: if write {
                Access::ReadWrite
            } else {
                Access::ReadOnly
            },
        }],
    });
    let context = resource_grants::grants(
        &db,
        &resources,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let tx = db.transaction().unwrap();
    let record = submissions::insert_in(
        &tx,
        conversation,
        Uuid::new_v4(),
        SubmissionInput {
            remote_compaction_v2: None,
            text: "Edit note".into(),
            user_input: vec![],
            skill_instructions: vec![],
            model: ModelIdentity {
                provider_id: "fixture".into(),
                model_id: "fixture".into(),
                revision: None,
            },
            reasoning_effort: None,
            service_tier: None,
            context,
            instructions: vec![],
            instruction_configs: vec![],
            skills: vec![],
        },
    )
    .unwrap();
    tx.commit().unwrap();
    db.execute(
        "insert into agent_text_tool_contracts(conversation_id,request_id,version) values(?1,?2,2)",
        params![
            record.conversation_id.to_string(),
            record.request_id.to_string()
        ],
    )
    .unwrap();
    drop(db);
    Fixture {
        _root: root,
        tools: TextTools {
            store,
            record,
            previous: Arc::new(RejectingToolExecutor),
            previous_names: Default::default(),
            previous_qualified_names: Default::default(),
            delivery_attempted: Default::default(),
            delivery_sealed: Default::default(),
            delivery_lease: Default::default(),
            cancellation: Default::default(),
            #[cfg(target_os = "macos")]
            file_workspace: None,
        },
        page,
        parent,
    }
}
fn path(id: Uuid) -> String {
    format!("source://notes/{id}")
}
async fn read(f: &Fixture, path: &str) -> Value {
    f.tools
        .read(Read {
            path: path.into(),
            start_char: 0,
            max_chars: 8000,
            view: None,
        })
        .await
        .unwrap()
}
fn patch(path: &str, old: &str, new: &str) -> String {
    let old = old.replace('\n', "\n-");
    let new = new.replace('\n', "\n+");
    format!("*** Begin Patch\n*** Update File: {path}\n@@\n-{old}\n+{new}\n*** End Patch")
}
fn decision(f: &Fixture, review: &decisions::Review, action: &str) -> decisions::Decision {
    decisions::Decision {
        conversation_id: f.tools.record.conversation_id,
        proposal_id: review.proposal_id.clone(),
        generation: review.generation,
        request_id: Uuid::new_v4(),
        action: action.into(),
    }
}
fn review(f: &Fixture) -> decisions::Review {
    decisions::review(
        &f.tools.store.connection().unwrap(),
        f.tools.record.conversation_id,
    )
    .unwrap()
    .unwrap()
}
#[tokio::test]
async fn current_body_saves_without_keeping_and_accept_undo_are_durable_and_idempotent() {
    let f = fixture(true);
    let target = path(f.page);
    let before = read(&f, &target).await;
    let first = patch(&target, "old", "new");
    let result = f.tools.apply("call-1", &first).await.unwrap();
    assert_eq!(result["targets"][0]["status"], "applied");
    assert_eq!(f.tools.apply("call-1", &first).await.unwrap(), result);
    assert!(
        read(&f, &target).await["text"]
            .as_str()
            .unwrap()
            .contains("new")
    );
    let saved = f
        .tools
        .read(Read {
            path: target.clone(),
            start_char: 0,
            max_chars: 8000,
            view: Some("saved".into()),
        })
        .await
        .unwrap();
    assert_ne!(saved["text"], before["text"]);
    assert!(saved["text"].as_str().unwrap().contains("new"));
    assert_eq!(
        review(&f).pages[0].before.as_deref(),
        before["text"].as_str()
    );
    read(&f, &target).await;
    f.tools
        .apply("call-2", &patch(&target, "new", "newer"))
        .await
        .unwrap();
    let shown = review(&f);
    let request = decision(&f, &shown, "accept");
    let request_id = request.request_id;
    let accepted = decisions::decide(&f.tools.store, request).unwrap();
    let repeated = decisions::decide(
        &f.tools.store,
        decisions::Decision {
            request_id,
            ..decision(&f, &shown, "accept")
        },
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&accepted).unwrap(),
        serde_json::to_value(repeated).unwrap()
    );
    assert_eq!(
        accepted.pages[0].document.blocks[0].props["privateSidecar"],
        "kept"
    );
    assert_eq!(
        accepted.pages[0].document.blocks[0].content["items"][0]["text"],
        "newer"
    );
    let saved_review = decisions::review_selected(
        &f.tools.store.connection().unwrap(),
        f.tools.record.conversation_id,
        None,
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(saved_review.state, "accept");
    assert_eq!(saved_review.generation, shown.generation);
    let snapshot = crate::hbs_storage::read(
        &f.tools.store.connection().unwrap(),
        "note",
        &f.page.to_string(),
    )
    .unwrap();
    assert_eq!(snapshot["saved"], snapshot["baseline"]);
    let undone = decisions::decide(&f.tools.store, decision(&f, &shown, "undo")).unwrap();
    assert_eq!(
        undone.pages[0].document.blocks[0].content["items"][0]["text"],
        "old"
    );
    let snapshot = crate::hbs_storage::read(
        &f.tools.store.connection().unwrap(),
        "note",
        &f.page.to_string(),
    )
    .unwrap();
    assert_eq!(
        snapshot["saved"], snapshot["baseline"],
        "whole Undo must settle the restored HBS baseline too"
    );
}
#[tokio::test]
async fn patches_use_current_content_despite_old_read_revisions_and_enforce_access() {
    let f = fixture(false);
    let target = path(f.page);
    read(&f, &target).await;
    let result = f
        .tools
        .apply("readonly", &patch(&target, "old", "new"))
        .await
        .unwrap();
    assert_eq!(result["targets"][0]["status"], "failed");
    assert!(
        note::pending(
            &f.tools.store.connection().unwrap(),
            f.tools.record.conversation_id
        )
        .unwrap()
        .is_empty()
    );
    let f = fixture(true);
    let target = path(f.page);
    read(&f, &target).await;
    f.tools
        .store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set revision=revision+1 where id=?1",
            [f.page.to_string()],
        )
        .unwrap();
    let result = f
        .tools
        .apply("stale", &patch(&target, "old", "new"))
        .await
        .unwrap();
    assert_eq!(result["targets"][0]["status"], "applied", "{result}");
    let mismatch = f
        .tools
        .apply("mismatched-context", &patch(&target, "old", "other"))
        .await
        .unwrap();
    assert_eq!(mismatch["targets"][0]["status"], "failed");
    assert!(
        f.tools
            .apply("invalid", "*** Begin Patch\n*** Delete File: bad\nINVALID")
            .await
            .is_err()
    );
}
#[tokio::test]
async fn creation_recycling_and_page_entrances_commit_as_one_group() {
    let f = fixture(true);
    let target = format!("notes/{}/new.md", f.parent);
    let patch = format!(
        "*** Begin Patch\n*** Add File: {target}\n+---\n+title: Child\n+---\n+\n+body\n*** End Patch"
    );
    let result = f.tools.apply("create", &patch).await.unwrap();
    assert_eq!(result["targets"][0]["status"], "applied", "{result}");
    let shown = review(&f);
    assert_eq!(shown.pages.len(), 2);
    let new = shown
        .pages
        .iter()
        .find(|page| page.kind == "create")
        .unwrap()
        .page_id
        .clone();
    let accepted = decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    assert!(
        accepted
            .pages
            .iter()
            .any(|page| page.page_id == new && page.parent_id.is_none())
    );
    let parent = accepted
        .pages
        .iter()
        .find(|page| page.page_id == f.parent.to_string())
        .unwrap();
    assert!(
        parent
            .document
            .blocks
            .iter()
            .any(|block| block.block_type == "researchObjectReference"
                && block.props["pageEntry"] == true
                && block.content["targetId"] == new)
    );
    let undone = decisions::decide(&f.tools.store, decision(&f, &shown, "undo")).unwrap();
    assert!(
        undone
            .pages
            .iter()
            .any(|page| page.page_id == new && page.status == "trashed")
    );
}
#[tokio::test]
async fn deleting_an_existing_note_is_reversible_without_resurrecting_unrelated_trash() {
    let f = fixture(true);
    let target = path(f.page);
    read(&f, &target).await;
    let result = f
        .tools
        .apply(
            "delete",
            &format!("*** Begin Patch\n*** Delete File: {target}\n*** End Patch"),
        )
        .await
        .unwrap();
    assert_eq!(result["targets"][0]["status"], "applied");
    let shown = review(&f);
    let accepted = decisions::decide(&f.tools.store, decision(&f, &shown, "accept")).unwrap();
    assert_eq!(accepted.pages[0].status, "trashed");
    let undone = decisions::decide(&f.tools.store, decision(&f, &shown, "undo")).unwrap();
    assert_eq!(undone.pages[0].status, "active");
}
#[tokio::test]
async fn a_mixed_batch_reports_note_success_and_file_failure_and_does_not_replay_unknown_calls() {
    let f = fixture(true);
    let target = path(f.page);
    read(&f, &target).await;
    let text = format!(
        "*** Begin Patch\n*** Add File: workspace/unregistered/file.txt\n+text\n*** Update File: {target}\n@@\n-old\n+new\n*** End Patch"
    );
    let result = f.tools.apply("mixed", &text).await.unwrap();
    assert_eq!(result["status"], "partial");
    assert_eq!(result["targets"][0]["status"], "failed");
    assert_eq!(result["targets"][1]["status"], "applied");
    assert_eq!(result, f.tools.apply("mixed", &text).await.unwrap());
    {
        let db = f.tools.store.connection().unwrap();
        assert!(
            journal::insert_sql(
                &db,
                f.tools.record.conversation_id,
                &format!("{}:lost", f.tools.record.request_id),
                &text,
            )
            .is_ok()
        );
    }
    let lost = f.tools.apply("lost", &text).await.unwrap();
    assert!(
        lost["targets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|target| target["status"] == "unknown")
    );
    assert!(
        f.tools
            .apply("mixed", &text.replace("+new", "+other"))
            .await
            .is_err()
    );
}
