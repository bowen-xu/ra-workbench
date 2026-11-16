use super::*;
use rusqlite::params;

use serde_json::{Value, json};

fn database() -> (Connection, Uuid) {
    let mut db = Connection::open_in_memory().unwrap();
    crate::storage::migrate(&mut db).unwrap();
    db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('paper','journalArticle','Paper','now','now')",[]).unwrap();
    let conversation = Uuid::new_v4();
    db.execute(
        "insert into agent_conversations(id,title) values(?1,'Internal task')",
        [conversation.to_string()],
    )
    .unwrap();
    (db, conversation)
}

#[test]
fn ordinary_conversation_configuration_recovery_has_no_internal_workflow_to_seed() {
    let (db, conversation) = database();
    seed_existing_context(
        None,
        None,
        &db,
        conversation,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert!(
        crate::agent::context_session::configuration::read_in(&db, conversation)
            .unwrap()
            .is_none()
    );
}

#[test]
fn portrait_generation_grants_its_own_template_and_persists_the_same_context_scope() {
    let (db, conversation) = database();
    let portrait = crate::structured_resources::find_by_paper(&db, "paper", "portrait").unwrap();
    let template = crate::structured_resources::ensure_paper_template(&db, "paper").unwrap();
    // Legacy imports can bind the portrait to its already allocated own template.
    db.execute(
        "update structured_resources set template_id=?2,template_revision=1 where id=?1",
        params![portrait.id, template.id],
    )
    .unwrap();
    let grants = paper_grants(
        &db,
        "paper",
        &[],
        conversation,
        "portrait",
        Some("frozen-source"),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let anchors = grants
        .iter()
        .map(|row| serde_json::from_str::<Value>(row.anchor.as_deref().unwrap()).unwrap())
        .collect::<Vec<_>>();
    let own = anchors
        .iter()
        .filter(|anchor| anchor["resource"]["resourceId"] == template.id)
        .collect::<Vec<_>>();
    assert_eq!(
        own.len(),
        1,
        "a readonly duplicate must not weaken the writable template target"
    );
    assert_eq!(own[0]["contextAccess"], "readWrite");
    assert!(
        anchors
            .iter()
            .all(|anchor| anchor["sourceSnapshotId"] == "frozen-source")
    );
    assert!(
        anchors
            .iter()
            .any(|anchor| anchor["resourceKind"] == "structuredReading")
    );
    let saved = crate::agent::context_session::configuration::read_in(&db, conversation)
        .unwrap()
        .unwrap();
    assert_eq!(saved.state["persistent"].as_array().unwrap().len(), 2);
    let parts = saved.state["persistent"][0]["resource"]["parts"]
        .as_array()
        .unwrap();
    assert_eq!(parts.len(), 3);
    assert!(parts.iter().all(|part| part["access"] == "readWrite"));
    assert_eq!(
        saved.state["persistent"][1]["resource"]["access"],
        "readOnly"
    );

    db.execute(
        "update agent_context_configurations set state_json=?2 where conversation_id=?1",
        params![
            conversation.to_string(),
            json!({"schemaVersion":1,"persistent":[],"temporary":[]}).to_string()
        ],
    )
    .unwrap();
    paper_grants(
        &db,
        "paper",
        &[],
        conversation,
        "portrait",
        None,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let saved = crate::agent::context_session::configuration::read_in(&db, conversation)
        .unwrap()
        .unwrap();
    assert!(saved.state["persistent"].as_array().unwrap().is_empty());
}

#[test]
fn reading_generation_pins_its_template_and_seeds_both_stage_targets() {
    let (db, conversation) = database();
    let reading =
        crate::structured_resources::find_by_paper(&db, "paper", "structuredReading").unwrap();
    let template = crate::structured_resources::ensure_paper_template(&db, "paper").unwrap();
    db.execute(
        "update structured_resources set template_id=?2,template_revision=1 where id=?1",
        params![reading.id, template.id],
    )
    .unwrap();
    let grants = paper_grants(
        &db,
        "paper",
        &[],
        conversation,
        "structuredReading",
        None,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(grants.len(), 4);
    let target_anchor: Value = serde_json::from_str(
        grants
            .iter()
            .find(|grant| grant.resource_id == reading.id)
            .unwrap()
            .anchor
            .as_deref()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        target_anchor["frozenTemplateRef"],
        json!({"resourceId":template.id,"revision":1})
    );
    let saved = crate::agent::context_session::configuration::read_in(&db, conversation)
        .unwrap()
        .unwrap();
    assert_eq!(saved.state["persistent"].as_array().unwrap().len(), 2);
    let parts = saved.state["persistent"][0]["resource"]["parts"]
        .as_array()
        .unwrap();
    assert!(parts.iter().any(|part| part["selected"] == true
        && part["resource"]["resourceId"] == template.id
        && part["access"] == "readWrite"));
    assert!(
        parts
            .iter()
            .any(|part| part["resource"]["resourceId"] == reading.id
                && part["access"] == "readWrite")
    );
}

#[test]
fn repeated_generation_uses_saved_permissions_selections_and_additional_context() {
    for kind in ["portrait", "structuredReading"] {
        let (db, conversation) = database();
        paper_grants(
            &db,
            "paper",
            &[],
            conversation,
            kind,
            None,
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap();
        let mut saved = crate::agent::context_session::configuration::read_in(&db, conversation)
            .unwrap()
            .unwrap();
        let parts = saved.state["persistent"][0]["resource"]["parts"]
            .as_array_mut()
            .unwrap();
        parts[0]["access"] = json!("readOnly");
        parts[0]["resource"]["access"] = json!("readOnly");
        for part in parts.iter_mut().skip(1) {
            part["selected"] = json!(false);
        }
        saved.state["persistent"]
            .as_array_mut()
            .unwrap()
            .truncate(1);
        saved.state["temporary"] = json!([{"label":"Additional metadata","resource":{"kind":"paperMetadata","resourceId":"paper"}}]);
        db.execute(
            "update agent_context_configurations set state_json=?2 where conversation_id=?1",
            params![conversation.to_string(), saved.state.to_string()],
        )
        .unwrap();
        let grants = paper_grants(
            &db,
            "paper",
            &[],
            conversation,
            kind,
            None,
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap();
        let anchors = grants
            .iter()
            .map(|grant| serde_json::from_str::<Value>(grant.anchor.as_deref().unwrap()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(anchors.len(), 2);
        assert!(
            anchors
                .iter()
                .all(|anchor| anchor["contextAccess"] == "readOnly")
        );
        assert!(
            anchors
                .iter()
                .any(|anchor| anchor["resource"]["kind"] == "paperMetadata")
        );
        let target = crate::structured_resources::find_by_paper(&db, "paper", kind).unwrap();
        assert!(require_writable_target(&grants, &target.id).is_err());
        assert_eq!(
            crate::agent::context_session::configuration::read_in(&db, conversation)
                .unwrap()
                .unwrap()
                .state,
            saved.state
        );

        db.execute(
            "update agent_context_configurations set state_json=?2 where conversation_id=?1",
            params![
                conversation.to_string(),
                json!({"schemaVersion":1,"persistent":[],"temporary":[]}).to_string()
            ],
        )
        .unwrap();
        assert!(
            paper_grants(
                &db,
                "paper",
                &[],
                conversation,
                kind,
                None,
                Some(crate::agent::writing_resources::test_sources::source())
            )
            .unwrap()
            .is_empty()
        );
    }
}

#[test]
fn configured_whole_paper_groups_resolve_effective_members_without_promoting_permissions() {
    let (db, conversation) = database();
    paper_grants(
        &db,
        "paper",
        &[],
        conversation,
        "portrait",
        None,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let state = json!({"schemaVersion":1,"persistent":[{"label":"Paper","resource":{
        "kind":"paperGroup","resourceId":"paper","parts":[],"newMemberAccess":"readWrite","groupAccess":"readWrite"
    }}],"temporary":[]});
    db.execute(
        "update agent_context_configurations set state_json=?2 where conversation_id=?1",
        params![conversation.to_string(), state.to_string()],
    )
    .unwrap();
    let target = crate::structured_resources::find_by_paper(&db, "paper", "portrait").unwrap();
    let grants = paper_grants(
        &db,
        "paper",
        &[],
        conversation,
        "portrait",
        None,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    require_writable_target(&grants, &target.id).unwrap();
    assert!(grants.iter().all(|grant| {
        crate::ai::structured_generation_support::prompt_resource(grant).is_some()
    }));
    assert!(require_writable_target(&grants, "another-paper-target").is_err());
    let mut readonly = state;
    readonly["persistent"][0]["resource"]["groupAccess"] = json!("readOnly");
    db.execute(
        "update agent_context_configurations set state_json=?2 where conversation_id=?1",
        params![conversation.to_string(), readonly.to_string()],
    )
    .unwrap();
    let grants = paper_grants(
        &db,
        "paper",
        &[],
        conversation,
        "portrait",
        None,
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert!(require_writable_target(&grants, &target.id).is_err());
    assert!(grants.iter().all(|grant| {
        serde_json::from_str::<Value>(grant.anchor.as_deref().unwrap()).unwrap()["contextAccess"]
            == "readOnly"
    }));
}
