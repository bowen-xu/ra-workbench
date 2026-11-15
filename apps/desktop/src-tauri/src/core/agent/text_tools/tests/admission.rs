use super::*;
use crate::agent::text_tools::note_review;

async fn create(f: &Fixture, key: &str) -> decisions::Review {
    let source = path(f.parent);
    let result = f.tools.apply(key, &format!("*** Begin Patch\n*** Add File: {source}\n+---\n+title: Generated page\n+---\n+\n+Generated content\n*** End Patch")).await.unwrap();
    assert_eq!(result["targets"][0]["status"], "applied", "{result}");
    review(f)
}
fn stale_cut(f: &Fixture, shown: &decisions::Review) -> (String, String, note::StoredNote) {
    let child = &shown
        .pages
        .iter()
        .find(|page| page.kind == "create")
        .unwrap()
        .page_id;
    let mut db = f.tools.store.connection().unwrap();
    let point = crate::hbs_storage::read(&db, "note", child).unwrap();
    crate::hbs_storage::test_sources::commit(
        &mut db,
        "note",
        child,
        crate::hbs_storage::Commit {
            id: "keep-body".into(),
            expected_version: point["version"].as_i64().unwrap(),
            saved: point["saved"].clone(),
            baseline: point["saved"].clone(),
        },
    )
    .unwrap();
    let old: String = db
        .query_row(
            "select candidate_json from agent_note_patch_candidates where page_id=?1",
            [child],
            |row| row.get(0),
        )
        .unwrap();
    let parent = crate::hbs_storage::read(&db, "note", &f.parent.to_string()).unwrap();
    let mut cut = parent["saved"].clone();
    cut["blocks"]
        .as_array_mut()
        .unwrap()
        .retain(|block| block["content"]["targetId"] != *child);
    crate::hbs_storage::test_sources::commit(
        &mut db,
        "note",
        &f.parent.to_string(),
        crate::hbs_storage::Commit {
            id: "cut".into(),
            expected_version: parent["version"].as_i64().unwrap(),
            saved: cut.clone(),
            baseline: cut,
        },
    )
    .unwrap();
    // Reproduce the deployed orphan mirror; all formal writes above used HBS.
    db.execute(
        "update agent_note_patch_candidates set candidate_json=?2,state='pending' where page_id=?1",
        params![child, old],
    )
    .unwrap();
    let actual = note::load(&db, child).unwrap();
    assert_eq!(actual.status, "trashed");
    (child.clone(), old, actual)
}
#[tokio::test]
async fn stale_child_neither_shadows_its_creation_source_nor_blocks_parent_read_or_create() {
    let f = fixture(true);
    let first = create(&f, "first").await;
    let (child, old, actual) = stale_cut(&f, &first);
    let frozen: String = f
        .tools
        .store
        .connection()
        .unwrap()
        .query_row(
            "select notes_json from agent_note_patch_snapshots where proposal_id=?1",
            [&first.proposal_id],
            |row| row.get(0),
        )
        .unwrap();
    // Any attempt to fence this unrelated owner fails without a UI transport.
    f.tools
        .store
        .text_editors()
        .register(Uuid::new_v4(), Some(child.clone()))
        .unwrap();
    for view in ["candidate", "saved"] {
        let read = f
            .tools
            .read(Read {
                path: path(f.parent),
                start_char: 0,
                max_chars: 8000,
                view: Some(view.into()),
            })
            .await
            .unwrap();
        assert_eq!(read["name"], "Parent");
    }
    create(&f, "second").await;
    let db = f.tools.store.connection().unwrap();
    let own = decisions::review_selected(
        &db,
        f.tools.record.conversation_id,
        None,
        Some(Uuid::parse_str(&child).unwrap()),
    )
    .unwrap()
    .unwrap();
    assert_eq!(own.proposal_id, first.proposal_id);
    let recovery = own
        .pages
        .iter()
        .find(|page| page.page_id == child)
        .unwrap()
        .recovery
        .as_ref()
        .unwrap();
    assert_eq!(recovery.current_revision, actual.revision);
    assert_eq!(recovery.status, "trashed");
    let retained: (String, String) = db
        .query_row(
            "select state,candidate_json from agent_note_patch_candidates where page_id=?1",
            [&child],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(retained, ("pending".into(), old));
    assert_eq!(
        serde_json::to_value(note::load(&db, &child).unwrap()).unwrap(),
        serde_json::to_value(actual).unwrap()
    );
    assert_eq!(
        db.query_row(
            "select notes_json from agent_note_patch_snapshots where proposal_id=?1",
            [&first.proposal_id],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        frozen
    );
}
#[tokio::test]
async fn unrelated_stale_proposal_is_preserved_while_an_addressed_atomic_group_still_rejects_staleness()
 {
    for same_group in [false, true] {
        let f = fixture(true);
        let initial = if same_group {
            format!(
                "*** Begin Patch\n*** Update File: {}\n@@\n-old\n+Candidate\n*** Update File: {}\n@@\n-parent\n+Parent candidate\n*** End Patch",
                path(f.page),
                path(f.parent)
            )
        } else {
            patch(&path(f.page), "old", "Candidate")
        };
        assert_eq!(
            f.tools.apply("initial", &initial).await.unwrap()["targets"][0]["status"],
            "applied"
        );
        let raw = {
            let db = f.tools.store.connection().unwrap();
            db.execute(
                "update note_pages set revision=revision+1 where id=?1",
                [f.page.to_string()],
            )
            .unwrap();
            db.query_row(
                "select candidate_json from agent_note_patch_candidates where page_id=?1",
                [f.page.to_string()],
                |row| row.get::<_, String>(0),
            )
            .unwrap()
        };
        let result = f
            .tools
            .apply(
                "next",
                &patch(
                    &path(f.parent),
                    if same_group {
                        "Parent candidate"
                    } else {
                        "parent"
                    },
                    "Next content",
                ),
            )
            .await
            .unwrap();
        assert_eq!(
            result["targets"][0]["status"],
            if same_group { "failed" } else { "applied" },
            "{result}"
        );
        if same_group {
            let message = result["targets"][0]["message"].as_str().unwrap();
            assert!(message.contains("MODEL_NOTE_PROPOSAL_STALE"));
            assert!(message.contains(&f.page.to_string()));
        }
        let db = f.tools.store.connection().unwrap();
        assert_eq!(
            db.query_row(
                "select candidate_json from agent_note_patch_candidates where page_id=?1",
                [f.page.to_string()],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            raw
        );
    }
}
#[tokio::test]
async fn explicit_retain_current_reconciles_only_the_selected_mirror_and_is_idempotent() {
    let f = fixture(true);
    let shown = create(&f, "first").await;
    let (child, _, actual) = stale_cut(&f, &shown);
    let before = {
        let db = f.tools.store.connection().unwrap();
        note::load(&db, &f.parent.to_string()).unwrap()
    };
    let request = note_review::Request {
        conversation_id: f.tools.record.conversation_id,
        proposal_id: shown.proposal_id.clone(),
        generation: shown.generation,
        request_id: Uuid::new_v4(),
        page_id: child.clone(),
        action: "retain-current".into(),
        chunk_id: None,
        document: Some(actual.document.clone()),
        history_request_id: None,
        direction: None,
        expected_revision: Some(actual.revision),
    };
    let mut stale = request.clone();
    stale.expected_revision = Some(actual.revision - 1);
    assert!(
        note_review::resolve_in(&f.tools.store, stale)
            .err()
            .unwrap()
            .contains("MODEL_NOTE_RECOVERY_STALE")
    );
    let first = note_review::resolve_in(&f.tools.store, request.clone()).unwrap();
    let again = note_review::resolve_in(&f.tools.store, request).unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(again).unwrap()
    );
    let db = f.tools.store.connection().unwrap();
    assert_eq!(
        serde_json::to_value(note::load(&db, &child).unwrap()).unwrap(),
        serde_json::to_value(actual).unwrap()
    );
    assert_eq!(
        serde_json::to_value(note::load(&db, &f.parent.to_string()).unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    assert_eq!(
        db.query_row(
            "select state from agent_note_patch_candidates where page_id=?1",
            [&child],
            |row| row.get::<_, String>(0)
        )
        .unwrap(),
        "accepted"
    );
}

#[tokio::test]
async fn retain_current_requires_confirmed_hbs_content_and_rolls_back_with_its_receipt() {
    let f = fixture(true);
    let output = f
        .tools
        .apply("candidate", &patch(&path(f.page), "old", "Candidate"))
        .await
        .unwrap();
    assert_eq!(output["targets"][0]["status"], "applied");
    let shown = review(&f);
    let id = f.page.to_string();
    let request = {
        let db = f.tools.store.connection().unwrap();
        let current = note::load(&db, &id).unwrap();
        note_review::Request {
            conversation_id: f.tools.record.conversation_id,
            proposal_id: shown.proposal_id.clone(),
            generation: shown.generation,
            request_id: Uuid::new_v4(),
            page_id: id.clone(),
            action: "retain-current".into(),
            chunk_id: None,
            document: Some(current.document),
            history_request_id: None,
            direction: None,
            expected_revision: Some(current.revision),
        }
    };
    assert!(
        note_review::resolve_in(&f.tools.store, request)
            .err()
            .unwrap()
            .contains("MODEL_NOTE_RECOVERY_SAVE_REQUIRED")
    );
    let (request, original) = {
        let mut db = f.tools.store.connection().unwrap();
        let original: String = db
            .query_row(
                "select candidate_json from agent_note_patch_candidates where page_id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        let point = crate::hbs_storage::read(&db, "note", &id).unwrap();
        crate::hbs_storage::test_sources::commit(
            &mut db,
            "note",
            &id,
            crate::hbs_storage::Commit {
                id: "explicit-hbs-keep".into(),
                expected_version: point["version"].as_i64().unwrap(),
                saved: point["saved"].clone(),
                baseline: point["saved"].clone(),
            },
        )
        .unwrap();
        db.execute("update agent_note_patch_candidates set candidate_json=?2,state='pending' where page_id=?1",params![id,original]).unwrap();
        let current = note::load(&db, &id).unwrap();
        db.execute_batch("create trigger fail_recovery_receipt before insert on agent_note_review_receipts begin select raise(abort,'synthetic recovery receipt failure');end;").unwrap();
        (
            note_review::Request {
                conversation_id: f.tools.record.conversation_id,
                proposal_id: shown.proposal_id.clone(),
                generation: shown.generation,
                request_id: Uuid::new_v4(),
                page_id: id.clone(),
                action: "retain-current".into(),
                chunk_id: None,
                document: Some(current.document),
                history_request_id: None,
                direction: None,
                expected_revision: Some(current.revision),
            },
            original,
        )
    };
    assert!(
        note_review::resolve_in(&f.tools.store, request.clone())
            .err()
            .unwrap()
            .contains("synthetic recovery receipt failure")
    );
    {
        let db = f.tools.store.connection().unwrap();
        assert_eq!(
            db.query_row(
                "select candidate_json from agent_note_patch_candidates where page_id=?1",
                [&id],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
            original
        );
        db.execute_batch("drop trigger fail_recovery_receipt")
            .unwrap();
    }
    note_review::resolve_in(&f.tools.store, request).unwrap();
}

#[tokio::test]
async fn page_recovery_does_not_materialize_purged_siblings_or_fall_back_to_the_conversation() {
    let f = fixture(true);
    let shown = create(&f, "create-before-purge").await;
    let (child, _, _) = stale_cut(&f, &shown);
    let db = f.tools.store.connection().unwrap();
    // Fault injection: a retained child snapshot refers to an identity whose
    // authority has been purged, reproducing QueryReturnedNoRows on inspection.
    let missing = Uuid::new_v4().to_string();
    db.execute("update agent_note_patch_candidates set candidate_json=json_set(candidate_json,'$.candidate.pageId',?2,'$.baseline.pageId',?2,'$.saved.pageId',?2)where page_id=?1",params![child,missing]).unwrap();
    assert!(decisions::review(&db, f.tools.record.conversation_id).is_err());
    assert!(
        decisions::review_selected(&db, f.tools.record.conversation_id, None, Some(f.parent))
            .unwrap()
            .is_none()
    );
    // A still-pending parent in the same atomic group must be inspected alone,
    // without loading the deleted sibling from its historical group snapshot.
    let parent = note::load(&db, &f.parent.to_string()).unwrap();
    let pending = note::PendingNote {
        path: path(f.parent),
        original_path: path(f.parent),
        scope_request_id: Uuid::new_v4(),
        baseline: Some(parent.clone()),
        candidate: Some(parent.clone()),
        saved: Some(parent),
        review_document: None,
        review_metadata: None,
        reviewed_metadata: true,
        manual_edited: false,
        imports: vec![],
    };
    db.execute("insert or replace into agent_note_patch_candidates(conversation_id,page_id,proposal_id,generation,state,baseline_json,candidate_json,persistence_version)values(?1,?2,?3,?4,'pending',?5,?5,2)",params![f.tools.record.conversation_id.to_string(),f.parent.to_string(),shown.proposal_id,shown.generation,serde_json::to_string(&pending).unwrap()]).unwrap();
    let own = decisions::review_selected(&db, f.tools.record.conversation_id, None, Some(f.parent))
        .unwrap()
        .unwrap();
    assert_eq!(own.pages.len(), 1);
    assert_eq!(own.pages[0].page_id, f.parent.to_string());
    assert!(own.pages[0].recovery.is_none());
}
