use super::*;
use research_agent_core::conversation::ConversationForkReader;

#[tokio::test]
async fn reading_note_branch_preserves_prefix_and_origin_without_page_scope_confusion() {
    let (db, catalog) = setup();
    {
        let mut connection = db.lock().unwrap();
        let tx = connection.transaction().unwrap();
        crate::notes::insert_reading_fixture(&tx, "note", "Retained note");
        tx.commit().unwrap();
    }
    let binding = ConversationBinding {
        kind: BindingKind::ReadingNote,
        resource_id: "note".into(),
    };
    catalog
        .bind_legacy_for_tests(id(1), binding.clone())
        .await
        .unwrap();
    let fork_request = ConversationForkRequest {
        binding: binding.clone(),
        ..request(2)
    };
    let branch = catalog
        .fork_legacy_for_tests(fork_request.clone())
        .await
        .unwrap();
    assert_eq!(branch.parent_end_seq, 5);
    assert_eq!(
        catalog.fork_legacy_for_tests(fork_request).await.unwrap(),
        branch
    );
    let reopened = SqliteConversationCatalog::new(db.clone());
    let origin = reopened
        .read_origin(id(2), binding.clone())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(origin.parent.id, id(1));
    assert_eq!(origin.parent_end_seq, 5);
    let exported = reopened.export_snapshot(id(2)).await.unwrap();
    assert_eq!(exported.turns.len(), 1);
    assert_eq!(exported.turns[0].items[0].text, "历史标记");
    // Even an explicit extra binding does not change the branch's original scope.
    catalog
        .bind_legacy_for_tests(id(2), request(2).binding.clone())
        .await
        .unwrap();
    assert_eq!(
        catalog
            .read_origin(id(2), request(2).binding)
            .await
            .unwrap_err(),
        CatalogError::NotFound
    );
    db.lock().unwrap().execute("delete from agent_conversation_bindings where conversation_id=?1 and resource_kind='readingNote'", [id(1).as_uuid().to_string()]).unwrap();
    assert_eq!(
        reopened.read_origin(id(2), binding).await.unwrap_err(),
        CatalogError::NotFound
    );
}
