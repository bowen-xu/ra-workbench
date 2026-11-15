use super::*;
use research_agent_core::conversation::ArchiveConversation;

#[tokio::test]
async fn each_pending_or_fresh_branch_freezes_archive_and_completes_only_after_its_receipt() {
    for branch in 0..3 {
        let f = Fixture::new("Original title");
        if branch > 0 {
            named_pending(&f, Uuid::from_u128(810), branch == 2).await;
        }
        f.store.connection().unwrap().execute_batch("create trigger reject_archive_receipt before insert on data_sql_operation_receipts when new.kind='agent.conversation.archive' begin select raise(abort,'synthetic receipt failure'); end;").unwrap();
        let original = f.legacy();
        assert!(matches!(
            migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &original).await,
            Err(Error::Mutation(_))
        ));
        let (conversation,status,expected):(String,String,u64)=f.store.connection().unwrap().query_row("select conversation_id,status,expected_event_count from agent_paper_reading_history_migrations",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap();
        let conversation = Uuid::parse_str(&conversation).unwrap();
        assert_eq!(status, "pending");
        assert!(
            !f.catalog
                .read(ConversationId::from_uuid(conversation), None)
                .await
                .unwrap()
                .archived
        );
        // Retry may not reinterpret either the frozen status or history extent.
        f.store.connection().unwrap().execute_batch("drop trigger reject_archive_receipt; update paper_ai_reading_sessions set status='failed';").unwrap();
        migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
            .await
            .unwrap();
        let command = ArchiveConversation {
            operation_id: Uuid::new_v5(&conversation, b"agent.paper_migration.archive:v1"),
            id: ConversationId::from_uuid(conversation),
            archived: true,
            expected_watermark: Some(expected),
        };
        let original = match f.catalog.archive_status(command.clone()).await.unwrap() {
            research_agent_core::conversation::ArchiveStatus::Committed(proof) => proof,
            _ => panic!("archive receipt missing"),
        };
        assert!(f.catalog.read(command.id, None).await.unwrap().archived);
        f.catalog
            .set_archived(ArchiveConversation {
                operation_id: Uuid::new_v4(),
                archived: false,
                expected_watermark: None,
                ..command.clone()
            })
            .await
            .unwrap();
        // Simulate crash between archive commit and mark_complete. The original
        // historical receipt must finish migration without overwriting restore.
        f.store
            .connection()
            .unwrap()
            .execute(
                "update agent_paper_reading_history_migrations set status='pending'",
                [],
            )
            .unwrap();
        migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
            .await
            .unwrap();
        assert!(!f.catalog.read(command.id, None).await.unwrap().archived);
        assert_eq!(
            f.catalog.archive_status(command).await.unwrap(),
            research_agent_core::conversation::ArchiveStatus::Committed(original)
        );
        migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy())
            .await
            .unwrap();
        assert!(
            !f.catalog
                .read(ConversationId::from_uuid(conversation), None)
                .await
                .unwrap()
                .archived
        );
        f.close();
    }
}

#[tokio::test]
async fn malformed_frozen_legacy_status_is_not_coerced_to_an_archive_decision() {
    let f = Fixture::new("Original");
    let conversation = Uuid::from_u128(811);
    named_pending(&f, conversation, true).await;
    f.store
        .connection()
        .unwrap()
        .execute(
            "update agent_paper_reading_legacy_sessions set status='unrecognized'",
            [],
        )
        .unwrap();
    assert!(matches!(
        migrate_session(&f.store, &f.catalog, &f.sessions, "paper", &f.legacy()).await,
        Err(Error::Corrupt)
    ));
    assert_eq!(
        f.store
            .connection()
            .unwrap()
            .query_row(
                "select status from agent_paper_reading_history_migrations",
                [],
                |row| row.get::<_, String>(0)
            )
            .unwrap(),
        "pending"
    );
    f.close();
}
