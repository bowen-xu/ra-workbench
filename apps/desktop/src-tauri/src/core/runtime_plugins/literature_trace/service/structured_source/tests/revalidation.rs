use super::*;

#[tokio::test]
async fn byte_only_parse_change_replays_build_even_when_decoded_input_and_snapshot_hash_match() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepared();
    let expected = prepared.projection.snapshot_id.clone();
    let raw: String = fixture
        .store
        .connection()
        .unwrap()
        .query_row(
            "select json_content from docling_parse_runs where id='parse-selected'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    fixture.configure(
        &[prepared.build.clone(), prepared.build, prepared.project],
        Some(0),
        false,
    );
    let caller = fixture.spawn();
    let entered = fixture.entered(0).await;
    if entered {
        fixture.raw_parse(&format!(" {raw} "));
    }
    fixture.release();
    let result = tokio::time::timeout(WAIT, caller).await.unwrap().unwrap();
    fixture.service.shutdown().await.unwrap();
    assert!(entered);
    result.unwrap();
    assert_eq!(fixture.proof()["snapshotId"], expected);
    assert_eq!(fixture.counts(), (1, 1));
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        3
    );
}

#[tokio::test]
async fn raw_parse_changes_during_native_wait_are_recaptured_before_publication() {
    let mut fixture = Fixture::new();
    let old = fixture.prepared();
    let raw: String = fixture
        .store
        .connection()
        .unwrap()
        .query_row(
            "select json_content from docling_parse_runs where id='parse-selected'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut changed: Value = serde_json::from_str(&raw).unwrap();
    changed["texts"][0]["text"] = json!("Changed current source");
    fixture.raw_parse(&changed.to_string());
    let new = fixture.prepared();
    fixture.raw_parse(&raw);
    let expected = new.projection.snapshot_id.clone();
    assert_ne!(old.projection.snapshot_id, expected);
    fixture.configure(&[old.build, new.build, new.project], Some(0), false);
    let caller = fixture.spawn();
    let entered = fixture.entered(0).await;
    if entered {
        fixture.raw_parse(&changed.to_string());
    }
    fixture.release();
    let result = tokio::time::timeout(WAIT, caller).await.unwrap().unwrap();
    fixture.service.shutdown().await.unwrap();
    assert!(entered);
    let value = result.unwrap();
    assert!(value.to_string().contains("Changed current source"));
    assert!(!value.to_string().contains("Authorized source"));
    assert_eq!(fixture.proof()["snapshotId"], expected);
    assert_eq!(fixture.counts(), (1, 1));
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        3
    );
}

#[tokio::test]
async fn authority_revocation_during_native_wait_prevents_all_publication() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepared();
    fixture.configure(&[prepared.build], Some(0), false);
    let caller = fixture.spawn();
    let entered = fixture.entered(0).await;
    if entered {
        fixture
            .store
            .connection()
            .unwrap()
            .execute(
                "update agent_conversations set archived=1 where id=?1",
                [fixture.record.conversation_id.to_string()],
            )
            .unwrap();
    }
    fixture.release();
    let result = tokio::time::timeout(WAIT, caller).await.unwrap().unwrap();
    fixture.service.shutdown().await.unwrap();
    assert!(entered);
    assert!(matches!(result, Err(SourceReadError::Context(_))));
    assert_eq!(fixture.counts(), (0, 0));
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        1
    );
}

#[tokio::test]
async fn attachment_source_hash_change_during_wait_cannot_reuse_old_parse() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepared();
    let raw: String = fixture
        .store
        .connection()
        .unwrap()
        .query_row(
            "select json_content from docling_parse_runs where id='parse-selected'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    {
        let db = fixture.store.connection().unwrap();
        db.execute(
            "update attachments set content_sha256=?1 where id='selected'",
            ["b".repeat(64)],
        )
        .unwrap();
    }
    let unavailable = fixture.build_only();
    fixture
        .store
        .connection()
        .unwrap()
        .execute(
            "update attachments set content_sha256=?1 where id='selected'",
            ["a".repeat(64)],
        )
        .unwrap();
    fixture.raw_parse(&raw);
    fixture.configure(&[prepared.build, unavailable], Some(0), false);
    let caller = fixture.spawn();
    let entered = fixture.entered(0).await;
    if entered {
        fixture
            .store
            .connection()
            .unwrap()
            .execute(
                "update attachments set content_sha256=?1 where id='selected'",
                ["b".repeat(64)],
            )
            .unwrap();
    }
    fixture.release();
    let result = tokio::time::timeout(WAIT, caller).await.unwrap().unwrap();
    fixture.service.shutdown().await.unwrap();
    assert!(entered);
    assert!(matches!(result, Err(SourceReadError::Context(_))));
    assert_eq!(fixture.counts(), (0, 0));
    assert_eq!(
        fixture
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        2
    );
}
