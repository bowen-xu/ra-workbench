//! Opt-in local diagnosis. Private snapshots are supplied at runtime, never fixtures
//! committed to the repository. Import occurs before a fresh SQL/log bootstrap.
use super::*;
use rusqlite::{params_from_iter, types::Value as SqlValue};
use sha2::{Digest, Sha256};
use std::path::Path;

fn private_directory(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
}

fn copy_assets(input: &Path, output: &Path, assets: &Value) {
    for object in assets["content_objects"].as_array().unwrap() {
        let hash = object["sha256"].as_str().unwrap();
        assert!(hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let relative = format!("{}/{}/{hash}", &hash[..2], &hash[2..4]);
        assert_eq!(object["relative_path"].as_str(), Some(relative.as_str()));
        let bytes = std::fs::read(input.join("objects").join(&relative)).unwrap();
        assert_eq!(format!("{:x}", Sha256::digest(&bytes)), hash);
        let destination = output.join("objects").join(relative);
        private_directory(destination.parent().unwrap());
        std::fs::write(destination, bytes).unwrap();
    }
}

fn import_rows(db: &Connection, table: &str, rows: &[Value]) {
    for row in rows {
        let row = row.as_object().expect("fixture row object");
        let columns = row.keys().collect::<Vec<_>>();
        assert!(
            columns
                .iter()
                .all(|key| key.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'))
        );
        let sql = format!(
            "insert into {table}({}) values({})",
            columns
                .iter()
                .map(|key| format!("\"{key}\""))
                .collect::<Vec<_>>()
                .join(","),
            vec!["?"; columns.len()].join(",")
        );
        let values = columns
            .iter()
            .map(|key| match &row[*key] {
                Value::Null => SqlValue::Null,
                Value::String(value) => SqlValue::Text(value.clone()),
                Value::Number(value) => SqlValue::Integer(value.as_i64().expect("integer field")),
                _ => panic!("unsupported fixture column type"),
            })
            .collect::<Vec<_>>();
        db.execute(&sql, params_from_iter(values))
            .expect("fixture row import failed");
    }
}

fn build_fixture(input: &Path, output: &Path) -> (AppStore, Uuid, Uuid) {
    assert!(!output.exists(), "choose a new isolated output directory");
    private_directory(output);
    let own: Value =
        serde_json::from_slice(&std::fs::read(input.join("private-note-snapshot.json")).unwrap())
            .unwrap();
    let related: Value =
        serde_json::from_slice(&std::fs::read(input.join("private-related-state.json")).unwrap())
            .unwrap();
    let assets: Value =
        serde_json::from_slice(&std::fs::read(input.join("private-assets.json")).unwrap()).unwrap();
    let page = Uuid::parse_str(own["noteId"].as_str().unwrap()).unwrap();
    let conversation =
        Uuid::parse_str(related["agent_conversations"][0]["id"].as_str().unwrap()).unwrap();
    let mut sql = Connection::open(output.join("research-assistant.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let tx = sql.transaction().unwrap();
    tx.execute_batch("pragma defer_foreign_keys=on;").unwrap();
    // Only these domain tables may be imported. Runtime pair identifiers, log
    // watermarks, provider settings, credentials and chat messages are excluded.
    for table in [
        "content_objects",
        "note_pages",
        "note_blocks",
        "hbs_note_baselines",
        "hbs_preservations",
        "hbs_commit_receipts",
        "note_revisions",
        "note_block_assets",
        "note_revision_assets",
        "hbs_note_baseline_assets",
        "hbs_preservation_assets",
        "agent_conversations",
        "agent_note_patch_candidates",
        "agent_note_patch_decisions",
    ] {
        for source in [&own["tables"], &related, &assets] {
            if let Some(rows) = source[table].as_array() {
                let rows = if table == "agent_conversations" {
                    rows.iter().map(|row|json!({"id":row["id"],"title":"Local recovery fixture","archived":row["archived"],"durable_count":0})).collect::<Vec<_>>()
                } else {
                    rows.clone()
                };
                import_rows(&tx, table, &rows);
            }
        }
    }
    let violations: i64 = tx
        .query_row("select count(*) from pragma_foreign_key_check", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(violations, 0, "fixture foreign key violations");
    tx.commit().unwrap();
    copy_assets(input, output, &assets);
    let store = AppStore::for_test_database(
        crate::data_interface::Database::open(sql, &output.join("data-log")).unwrap(),
        output.join("objects"),
    );
    (store, page, conversation)
}

fn tools_for_page(store: AppStore, page: Uuid, conversation: Uuid) -> TextTools {
    let mut db = store.connection().unwrap();
    let resources = [Resource::NoteGroup {
        resource_id: page.to_string(),
        new_member_access: None,
        group_access: None,
        parts: vec![Part {
            resource: Resource::Note { resource_id: page },
            label: "Local recovery fixture".into(),
            selected: true,
            access: Access::ReadWrite,
        }],
    }];
    let context = resource_grants::grants(&db, &resources, None).unwrap();
    let tx = db.transaction().unwrap();
    let record = submissions::insert_in(
        &tx,
        conversation,
        Uuid::new_v4(),
        SubmissionInput {
            remote_compaction_v2: None,
            text: "Local recovery verification".into(),
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
        "insert into agent_text_tool_contracts(conversation_id,request_id,version)values(?1,?2,2)",
        params![conversation.to_string(), record.request_id.to_string()],
    )
    .unwrap();
    drop(db);
    TextTools {
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
    }
}

#[tokio::test]
#[ignore = "Requires explicitly authorized private local snapshots and a new isolated output directory"]
async fn inspect_local_note_recovery_snapshot() {
    let input = std::path::PathBuf::from(
        std::env::var_os("NOTE_RECOVERY_INPUT").expect("NOTE_RECOVERY_INPUT required"),
    );
    let output = std::path::PathBuf::from(
        std::env::var_os("NOTE_RECOVERY_OUTPUT").expect("NOTE_RECOVERY_OUTPUT required"),
    );
    let temporary = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../.tmp")
        .canonicalize()
        .unwrap();
    assert!(
        output.is_absolute() && !output.exists(),
        "choose a new absolute output path"
    );
    assert!(
        output
            .parent()
            .unwrap()
            .canonicalize()
            .unwrap()
            .starts_with(temporary)
    );
    assert!(
        !output
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    );
    private_directory(&output);
    let (store, page, conversation) = build_fixture(&input, &output.join("inspection"));
    {
        let db = store.connection().unwrap();
        let snapshot = crate::hbs_storage::read(&db, "note", &page.to_string()).unwrap();
        std::fs::write(
            output.join("private-authority.json"),
            serde_json::to_vec(&snapshot).unwrap(),
        )
        .unwrap();
        // This is the legacy conversation fallback that produced the reported
        // missing-row error, using the retained real orphan snapshot unchanged.
        let legacy_error = decisions::review(&db, conversation)
            .err()
            .expect("legacy fallback must fail for the deleted sibling");
        assert!(
            legacy_error.contains("Query returned no rows"),
            "legacy fallback failed for a different reason"
        );
        assert!(
            decisions::review_selected(&db, conversation, None, Some(page))
                .unwrap()
                .is_none()
        );
        assert!(
            snapshot["saved"] == snapshot["baseline"],
            "authority has pending content"
        );
    }
    drop(store);
    // Reopen the generated SQL/log pair after migration. Native UI acceptance
    // separately exercises the complete application's startup boundary.
    let mut sql = Connection::open(output.join("inspection/research-assistant.sqlite3")).unwrap();
    crate::storage::migrate(&mut sql).unwrap();
    let reopened =
        crate::data_interface::Database::open(sql, &output.join("inspection/data-log")).unwrap();
    assert!(reopened.logged());
    drop(reopened);
    let (store, page, conversation) = build_fixture(&input, &output.join("agent-write"));
    let tools = tools_for_page(store, page, conversation);
    let result=tools.apply("local-recovery-create",&format!("*** Begin Patch\n*** Add File: {}\n+---\n+title: Local recovery child\n+---\n+\n+Synthetic verification content\n*** End Patch",path(page))).await.unwrap();
    assert_eq!(
        result["targets"][0]["status"], "applied",
        "agent creation was not admitted"
    );
}
