use super::*;
use crate::agent::repository::create_conversation;
use research_agent_core::{conversation::ConversationId, protocol::ModelIdentity};
use sha2::Digest;

fn input() -> SubmissionInput {
    SubmissionInput {
        remote_compaction_v2: None,
        user_input: vec![],
        skill_instructions: vec![],
        instructions: vec![],
        instruction_configs: vec![],
        skills: vec![],
        text: "Explain this paragraph".into(),
        model: ModelIdentity {
            provider_id: "fixture".into(),
            model_id: "test".into(),
            revision: Some("v1".into()),
        },
        reasoning_effort: Some("high".into()),
        service_tier: None,
        context: vec![ContextText {
            source_id: "S1".into(),
            kind: ContextSourceKind::Manuscript,
            resource_id: "draft/main.tex".into(),
            revision: "buffer:42".into(),
            anchor: Some("line:8".into()),
            text: "Frozen manuscript text".into(),
        }],
    }
}
fn setup() -> (Arc<Mutex<Connection>>, SqliteSubmissionStore) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 69);
    create_conversation(
        &mut db,
        ConversationId::from_uuid(Uuid::from_u128(1)),
        "test",
    )
    .unwrap();
    let db = Arc::new(Mutex::new(db));
    (db.clone(), SqliteSubmissionStore::new(db))
}

#[tokio::test]
async fn retries_and_independent_owners_return_the_original_frozen_record() {
    let (db, store) = setup();
    let id = Uuid::from_u128(1);
    let request = Uuid::from_u128(2);
    let other = SqliteSubmissionStore::new(db.clone());
    let (a, b) = tokio::join!(
        store.put(id, request, input()),
        other.put(id, request, input())
    );
    let record = a.unwrap();
    assert_eq!(record, b.unwrap());
    assert_eq!(store.get_by_turn(id, record.turn_id).await.unwrap(), record);
    assert_eq!(
        store.get_by_turn(Uuid::from_u128(99), record.turn_id).await,
        Err(SubmissionError::NotFound)
    );
    drop(store);
    assert_eq!(other.get(id, request).await.unwrap(), record);
    assert_eq!(record.input.context[0].revision, "buffer:42");
    let db = db.lock().unwrap();
    assert_eq!(
        db.query_row("select count(*) from agent_submissions", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        db.query_row("select durable_count from agent_conversations", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap(),
        0
    );
}

#[tokio::test]
async fn changed_input_model_and_context_cannot_reuse_an_acknowledged_identity() {
    let (_, store) = setup();
    let id = Uuid::from_u128(1);
    let request = Uuid::from_u128(2);
    let original = store.put(id, request, input()).await.unwrap();
    for variant in 0..4 {
        let mut changed = input();
        match variant {
            0 => changed.text.push('!'),
            1 => changed.model.model_id = "different".into(),
            2 => changed.context[0].revision = "buffer:43".into(),
            _ => changed.reasoning_effort = None,
        }
        assert_eq!(
            store.put(id, request, changed).await,
            Err(SubmissionError::Conflict)
        );
        assert_eq!(store.get(id, request).await.unwrap(), original);
    }
}

#[tokio::test]
async fn missing_conversation_invalid_input_and_corruption_are_not_empty_success() {
    let (db, store) = setup();
    let id = Uuid::from_u128(1);
    let request = Uuid::from_u128(2);
    assert_eq!(
        store.put(Uuid::from_u128(99), request, input()).await,
        Err(SubmissionError::NotFound)
    );
    let mut invalid = input();
    invalid.context.push(invalid.context[0].clone());
    assert_eq!(
        store.put(id, request, invalid).await,
        Err(SubmissionError::InvalidInput)
    );
    assert_eq!(store.get(id, request).await, Err(SubmissionError::NotFound));
    store.put(id, request, input()).await.unwrap();
    db.lock()
        .unwrap()
        .execute("update agent_submissions set input_json = '{}'", [])
        .unwrap();
    assert_eq!(store.get(id, request).await, Err(SubmissionError::Corrupt));
    assert_eq!(
        store.put(id, request, input()).await,
        Err(SubmissionError::Corrupt)
    );
}

#[tokio::test]
async fn disk_reopen_retains_input_and_request_ids_are_conversation_scoped() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("submissions.sqlite");
    let mut db = Connection::open(&path).unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 69);
    for n in [1, 2] {
        create_conversation(
            &mut db,
            ConversationId::from_uuid(Uuid::from_u128(n)),
            "test",
        )
        .unwrap();
    }
    let store = SqliteSubmissionStore::new(Arc::new(Mutex::new(db)));
    let request = Uuid::from_u128(3);
    let mut with_skill = input();
    with_skill
        .skills
        .push(research_agent_core::skills::SubmissionSkill {
            descriptor: research_agent_core::skills::SkillDescriptor {
                identity: research_agent_core::skills::SkillIdentity {
                    environment_id: "paper".into(),
                    source_id: "review/SKILL.md".into(),
                    revision: "v1".into(),
                },
                name: "Review".into(),
                description: "Review manuscript".into(),
                allow_implicit_invocation: false,
            },
            explicit: true,
        });
    let first = store
        .put(Uuid::from_u128(1), request, with_skill.clone())
        .await
        .unwrap();
    let second = store
        .put(Uuid::from_u128(2), request, input())
        .await
        .unwrap();
    assert_ne!(first.turn_id, second.turn_id);
    assert_ne!(first.context_snapshot_id, second.context_snapshot_id);
    drop(store);
    let db = Connection::open(&path).unwrap();
    db.execute_batch("pragma foreign_keys = on").unwrap();
    let reopened = SqliteSubmissionStore::new(Arc::new(Mutex::new(db)));
    assert_eq!(
        reopened.get(Uuid::from_u128(1), request).await.unwrap(),
        first
    );
    with_skill.skills[0].descriptor.identity.revision = "v2".into();
    assert_eq!(
        reopened.put(Uuid::from_u128(1), request, with_skill).await,
        Err(SubmissionError::Conflict)
    );
    assert_eq!(
        reopened
            .put(Uuid::from_u128(2), request, input())
            .await
            .unwrap(),
        second
    );
}

#[tokio::test]
async fn instruction_snapshot_is_durable_and_part_of_retry_identity() {
    use research_agent_core::instructions::*;
    let (db, store) = setup();
    let id = Uuid::from_u128(1);
    let request = Uuid::new_v4();
    let mut draft = input();
    draft.instructions = assemble_instructions(
        vec![InstructionDocument {
            environment_id: "paper".into(),
            scope_id: ".".into(),
            source_id: "AGENTS.md".into(),
            revision: "v1".into(),
            text: "Style rule".into(),
        }],
        1024,
    )
    .unwrap();
    let saved = store.put(id, request, draft.clone()).await.unwrap();
    drop(store);
    let reopened = SqliteSubmissionStore::new(db);
    assert_eq!(reopened.get(id, request).await.unwrap(), saved);
    assert_eq!(
        reopened.put(id, request, draft.clone()).await.unwrap(),
        saved
    );
    draft.instructions[0].revision = "v2".into();
    assert_eq!(
        reopened.put(id, request, draft).await,
        Err(SubmissionError::Conflict)
    );
}

#[test]
fn image_manifest_read_is_submission_scoped_and_ordered() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 95);
    let conversation = Uuid::from_u128(10);
    let request = Uuid::from_u128(11);
    create_conversation(&mut db, ConversationId::from_uuid(conversation), "test").unwrap();
    db.execute("insert into agent_submissions(conversation_id,request_id,turn_id,context_snapshot_id,schema_version,input_json) values(?1,?2,?3,?4,1,'{}')", rusqlite::params![conversation.to_string(),request.to_string(),Uuid::from_u128(12).to_string(),Uuid::from_u128(13).to_string()]).unwrap();
    db.execute(
        "insert into agent_submission_images values(?1,?2,'second',?3,'image/png',2,1)",
        rusqlite::params![
            conversation.to_string(),
            request.to_string(),
            "b".repeat(64)
        ],
    )
    .unwrap();
    db.execute(
        "insert into agent_submission_images values(?1,?2,'first',?3,'image/png',1,0)",
        rusqlite::params![
            conversation.to_string(),
            request.to_string(),
            "a".repeat(64)
        ],
    )
    .unwrap();
    let refs = read_image_manifest(&db, conversation, request).unwrap();
    assert_eq!(
        refs.iter()
            .map(|r| r.source_id.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second"]
    );
    assert!(
        read_image_manifest(&db, Uuid::from_u128(99), request)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn retry_frozen_input_creates_a_new_turn_and_copies_images() {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage_migration_tests::migrate_through(&mut db, 95);
    let conversation = Uuid::from_u128(20);
    let source_request = Uuid::from_u128(21);
    let retry_request = Uuid::from_u128(22);
    create_conversation(&mut db, ConversationId::from_uuid(conversation), "test").unwrap();
    let tx = db.transaction().unwrap();
    let source = insert_in(&tx, conversation, source_request, input()).unwrap();
    tx.execute(
        "insert into agent_submission_images values(?1,?2,'figure',?3,'image/png',7,0)",
        rusqlite::params![
            conversation.to_string(),
            source_request.to_string(),
            "a".repeat(64)
        ],
    )
    .unwrap();
    let retry = retry_frozen_in(&tx, conversation, source_request, retry_request).unwrap();
    tx.commit().unwrap();

    assert_eq!(retry.input, source.input);
    assert_ne!(retry.turn_id, source.turn_id);
    assert_ne!(retry.context_snapshot_id, source.context_snapshot_id);
    let copied: (String, String, i64) = db
        .query_row(
            "select source_id,content_sha256,ordinal from agent_submission_images where conversation_id=?1 and request_id=?2",
            rusqlite::params![conversation.to_string(), retry_request.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(copied, ("figure".into(), "a".repeat(64), 0));
    assert!(read(&db, conversation, source_request).unwrap().is_some());
}

#[test]
fn materialize_image_manifest_verifies_content_store_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = b"\x89PNG\r\n\x1a\nrecovery".to_vec();
    let hash = format!("{:x}", sha2::Sha256::digest(&bytes));
    let relative = format!(
        "ai-reading-images/{}/{hash2}/{hash}.png",
        &hash[..2],
        hash2 = &hash[2..4]
    );
    let path = dir.path().join(&relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let refs = vec![SubmissionImageReference {
        source_id: "paper-source:1".into(),
        content_sha256: hash.clone(),
        media_type: "image/png".into(),
        byte_size: bytes.len() as u64,
        ordinal: 0,
    }];
    let images = materialize_image_manifest(dir.path(), &refs).unwrap();
    assert_eq!(images[0].bytes, bytes);
    std::fs::write(path, vec![0; bytes.len()]).unwrap();
    assert_eq!(
        materialize_image_manifest(dir.path(), &refs),
        Err(SubmissionError::Corrupt)
    );
}

#[tokio::test]
async fn checkpoint_image_subset_keeps_provenance_and_skips_unreferenced_objects() {
    let root = tempfile::tempdir().unwrap();
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    let id = Uuid::new_v4();
    create_conversation(&mut db, ConversationId::from_uuid(id), "retained images").unwrap();
    let app = crate::storage::AppStore::for_test(db, root.path().join("objects"));
    let store = SqliteSubmissionStore::new(app.shared_agent_connection());
    let record = store.put(id, Uuid::new_v4(), input()).await.unwrap();
    let bytes = b"\x89PNG\r\n\x1a\nretained".to_vec();
    let hash = format!("{:x}", Sha256::digest(&bytes));
    let missing = "a".repeat(64);
    let path = app.content_directory().join(format!(
        "ai-reading-images/{}/{}/{hash}.png",
        &hash[..2],
        &hash[2..4]
    ));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    {
        let connection = app.shared_agent_connection();
        let mut db = connection.lock().unwrap();
        let tx = db.transaction().unwrap();
        for (ordinal, (source, sha, size)) in
            [("retained", &hash, bytes.len()), ("covered", &missing, 12)]
                .into_iter()
                .enumerate()
        {
            tx.execute("insert into agent_submission_images(conversation_id,request_id,source_id,content_sha256,media_type,byte_size,ordinal) values(?1,?2,?3,?4,'image/png',?5,?6)",
                params![id.to_string(), record.request_id.to_string(), source, sha, size as i64, ordinal as i64]).unwrap();
        }
        tx.commit().unwrap();
    }
    let materializer = AppImageMaterializer::new(&app);
    let selected = materializer
        .materialize_history_images(id, record.turn_id, vec![hash.clone()])
        .await
        .unwrap();
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].bytes, bytes);
    assert!(
        materializer
            .materialize_history(id, record.turn_id)
            .await
            .is_err()
    );
    assert!(
        materializer
            .materialize_history_images(Uuid::new_v4(), record.turn_id, vec![hash.clone()])
            .await
            .is_err()
    );
    assert!(
        materializer
            .materialize_history_images(id, record.turn_id, vec!["b".repeat(64)])
            .await
            .unwrap()
            .is_empty()
    );
    std::fs::write(path, vec![0; bytes.len()]).unwrap();
    assert_eq!(
        materializer
            .materialize_history_images(id, record.turn_id, vec![hash])
            .await,
        Err(SubmissionError::Corrupt)
    );
}
