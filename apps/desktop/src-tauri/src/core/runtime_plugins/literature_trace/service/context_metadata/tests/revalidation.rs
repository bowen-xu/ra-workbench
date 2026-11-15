use super::*;

#[tokio::test]
async fn each_current_metadata_input_change_during_native_wait_requires_another_projection() {
    for (sql, before, after) in [
        (
            "update bibliographic_items set title=?1 where id='paper'",
            "Metadata Ω",
            "Changed title Ω",
        ),
        (
            "update creators set given_name=?1 where id='creator'",
            "Given",
            "Changed given Ω",
        ),
        (
            "update identifiers set display_value=?1 where id='identifier'",
            "https://example.invalid/Ω",
            "Changed identifier Ω",
        ),
    ] {
        let mut fixture = Fixture::new();
        let old = fixture.response();
        fixture
            .store
            .connection()
            .unwrap()
            .execute(sql, [after])
            .unwrap();
        let new = fixture.response();
        fixture
            .store
            .connection()
            .unwrap()
            .execute(sql, [before])
            .unwrap();
        fixture.configure(&[old, new], Some(0), false);
        let caller = fixture.spawn(fixture.metadata_read());
        let entered = fixture.entered(0).await;
        if entered {
            fixture
                .store
                .connection()
                .unwrap()
                .execute(sql, [after])
                .unwrap();
        }
        fixture.release();
        let output = tokio::time::timeout(WAIT, caller).await.unwrap().unwrap();
        fixture.service.shutdown().await.unwrap();
        assert!(entered, "the actual exchange must be held before mutation");
        assert!(!output.is_error, "{}", output.content);
        assert!(output.content.contains(after));
        assert_eq!(
            fixture.calls(),
            2,
            "changed capture must not reuse the old projected body"
        );
        assert_eq!(fixture.proofs(), 0);
    }
}

#[tokio::test]
async fn mixed_search_rechecks_other_core_text_after_metadata_wait_instead_of_returning_its_old_prefix()
 {
    let mut fixture = Fixture::new();
    fixture.mixed(true);
    let response = fixture.response();
    fixture.configure(&[response], Some(0), false);
    let caller = fixture.spawn(fixture.search());
    let entered = fixture.entered(0).await;
    if entered {
        fixture.change_note("Current changed Note Ω");
    }
    fixture.release();
    let output = tokio::time::timeout(WAIT, caller).await.unwrap().unwrap();
    fixture.service.shutdown().await.unwrap();
    assert!(entered);
    assert!(!output.is_error, "{}", output.content);
    assert!(output.content.contains("Current changed Note Ω"));
    assert_eq!(
        fixture.calls(),
        1,
        "unchanged metadata stays valid while the Core prefix is reread"
    );
    assert_eq!(fixture.proofs(), 2);
}

#[tokio::test]
async fn conversation_revocation_during_native_wait_prevents_a_successful_metadata_result() {
    let mut fixture = Fixture::new();
    let response = fixture.response();
    fixture.configure(&[response], Some(0), false);
    let caller = fixture.spawn(fixture.metadata_read());
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
    let output = tokio::time::timeout(WAIT, caller).await.unwrap().unwrap();
    fixture.service.shutdown().await.unwrap();
    assert!(entered && output.is_error);
    let value: Value = serde_json::from_str(&output.content).unwrap();
    assert_eq!(value["error"], "independent conversation unavailable");
    assert!(!output.content.contains("Metadata Ω"));
    assert_eq!(fixture.calls(), 1);
    assert_eq!(fixture.proofs(), 0);
}
