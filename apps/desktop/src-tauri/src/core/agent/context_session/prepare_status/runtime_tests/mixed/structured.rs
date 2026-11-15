//! Core structured bodies retain their original journal during Native manuscript reads.
use super::*;
use research_agent_core::{runner::ToolExecutor, submission::SubmissionRecord};
use sha2::{Digest, Sha256};

fn executor(store: AppStore, record: SubmissionRecord) -> Arc<dyn ToolExecutor> {
    crate::agent::context_session::resource_tools::registration(
        store,
        record,
        OwnedToolRegistration::new(vec![], Arc::new(RejectingToolExecutor)),
    )
    .into_parts()
    .1
}

#[tokio::test]
#[ignore = "requires actual independent writing package; Core structured and manuscript reads"]
async fn actual_package_mixed_structured_reads_preserve_original_bodies_and_proofs() {
    for manuscript_first in [false, true] {
        let mut f = writing_test_fixture(false);
        let native = binding();
        let conversation = writing_test_request(&f, Uuid::new_v4()).conversation_id;
        let page = note(&mut f.store.connection().unwrap());
        let objects = {
            let db = f.store.connection().unwrap();
            db.execute("insert into bibliographic_items(id,item_type,title,created_at,updated_at) values('structured-paper','document','Structured sources','now','now')", []).unwrap();
            let objects =
                crate::structured_resources::ensure_paper_resources(&db, "structured-paper")
                    .unwrap();
            for object in &objects {
                let body = json!({"description":format!("Frozen {} Ω", object.kind)});
                db.execute(
                    "update structured_resources set content_json=?1,baseline_json=?1 where id=?2",
                    rusqlite::params![body.to_string(), object.id],
                )
                .unwrap();
            }
            objects
        };
        let static_store = f.store.clone();
        f.store = f
            .store
            .without_static_writing_context_sources()
            .with_native_writing_context(native.clone());
        let mut input = request(conversation, f.manuscript);
        let manuscript = input.resources.remove(0);
        input.resources = vec![
            Resource::Note { resource_id: page },
            // The original legacy grant expands to the same Core portrait object.
            Resource::PaperPortrait {
                resource_id: "structured-paper".into(),
            },
        ];
        input
            .resources
            .extend(objects.iter().map(|object| Resource::StructuredObject {
                resource_id: object.id.clone(),
                access: Some(Access::ReadOnly),
            }));
        input.resources.push(Resource::TemplateCatalog {
            resource_id: crate::agent::context_session::structured::TEMPLATE_CATALOG_ID.into(),
            access: Some(Access::ReadOnly),
        });
        if manuscript_first {
            input.resources.insert(0, manuscript);
        } else {
            input.resources.push(manuscript);
        }
        prepare(&f.store, &f.models, input.clone(), None)
            .await
            .unwrap();
        let record = crate::agent::context_session::restore(
            &f.store,
            crate::agent::context_session::Receipt {
                conversation_id: conversation,
                request_id: input.request_id,
            },
        )
        .await
        .unwrap();
        let tools = executor(f.store.clone(), record.clone());
        let original = executor(static_store, record.clone());
        let search = call(
            "research_search_sources",
            "structured-search",
            json!({"query":"Frozen"}),
        );
        assert_eq!(
            f.store
                .connection()
                .unwrap()
                .query_row(
                    "select count(*) from agent_text_read_scopes where scope_request_id=?1",
                    [input.request_id.to_string()],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );
        let output = tools.authorize_and_execute(search.clone()).await.unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert_eq!(
            f.store
                .connection()
                .unwrap()
                .query_row(
                    "select count(*) from agent_text_read_scopes where scope_request_id=?1",
                    [input.request_id.to_string()],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            4
        );
        let expected = original
            .authorize_and_execute(search.clone())
            .await
            .unwrap();
        assert!(!expected.is_error, "{}", expected.content);
        let value: Value = serde_json::from_str(&output.content).unwrap();
        assert_eq!(
            value,
            serde_json::from_str::<Value>(&expected.content).unwrap()
        );
        let matched: std::collections::BTreeSet<_> = value["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["sourceId"].as_str().unwrap())
            .collect();
        assert_eq!(
            matched.len(),
            5,
            "Note, manuscript and three structured objects"
        );
        let proofs = || -> Vec<(String, String, String)> {
            let db = f.store.connection().unwrap();
            let mut query = db.prepare("select path,version,snapshot_json from agent_text_read_scopes where conversation_id=?1 and scope_request_id=?2 order by path").unwrap();
            query
                .query_map(
                    rusqlite::params![conversation.to_string(), input.request_id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        let first_proofs = proofs();
        assert_eq!(
            first_proofs.len(),
            4,
            "Core Note and three original structured journals"
        );
        for object in &objects {
            let source = crate::agent::context_session::resource_grants::grants(
                &f.store.connection().unwrap(),
                &[Resource::StructuredObject {
                    resource_id: object.id.clone(),
                    access: Some(Access::ReadOnly),
                }],
                None,
            )
            .unwrap()
            .remove(0);
            let read = tools
                .authorize_and_execute(call(
                    "research_read_source",
                    &object.id,
                    json!({"sourceId":source.source_id}),
                ))
                .await
                .unwrap();
            assert!(!read.is_error, "{}", read.content);
            let read: Value = serde_json::from_str(&read.content).unwrap();
            assert!(
                read["text"]
                    .as_str()
                    .unwrap()
                    .contains(&format!("Frozen {} Ω", object.kind))
            );
        }
        let repeated = tools.authorize_and_execute(search.clone()).await.unwrap();
        assert!(!repeated.is_error, "{}", repeated.content);
        assert_eq!(proofs(), first_proofs);
        let portrait = objects
            .iter()
            .find(|object| object.kind == "portrait")
            .unwrap();
        f.store
            .connection()
            .unwrap()
            .execute(
                "update structured_resources set content_json=?1,revision=revision+1 where id=?2",
                rusqlite::params![
                    json!({"description":"Current portrait Ω"}).to_string(),
                    portrait.id
                ],
            )
            .unwrap();
        let current = tools
            .authorize_and_execute(call(
                "research_search_sources",
                "current-structured",
                json!({"query":"Current portrait"}),
            ))
            .await
            .unwrap();
        assert!(!current.is_error, "{}", current.content);
        assert!(current.content.contains("Current portrait Ω"));
        let current_proofs = proofs();
        assert_eq!(current_proofs.len(), first_proofs.len());
        assert_eq!(
            current_proofs
                .iter()
                .zip(&first_proofs)
                .filter(|(new, old)| new != old)
                .count(),
            1
        );
        let raw = "{\"description\":\"Private draft Ω\"}";
        f.store.connection().unwrap().execute(
            "insert into structured_resource_drafts(id,resource_id,creator_scope,creator_conversation_id,base_revision,revision,content,content_hash) select ?1,id,?2,?2,revision,1,?3,?4 from structured_resources where id=?5",
            rusqlite::params![Uuid::new_v4().to_string(), conversation.to_string(), raw, format!("{:x}",Sha256::digest(raw)),portrait.id],
        ).unwrap();
        let draft = tools
            .authorize_and_execute(call(
                "research_search_sources",
                "private-structured-draft",
                json!({"query":"Private draft"}),
            ))
            .await
            .unwrap();
        assert!(!draft.is_error, "{}", draft.content);
        assert!(draft.content.contains("Private draft Ω"));
        let next_proofs = proofs();
        assert_eq!(next_proofs.len(), first_proofs.len());
        assert_eq!(
            next_proofs
                .iter()
                .zip(&current_proofs)
                .filter(|(new, old)| new != old)
                .count(),
            1
        );
        assert_eq!(
            f.store
                .connection()
                .unwrap()
                .query_row(
                    "select content_json from structured_resources where id=?1",
                    [&portrait.id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            json!({"description":"Current portrait Ω"}).to_string()
        );
        let mut missing = native.clone();
        missing.package_key = "0".repeat(64);
        let unavailable = executor(
            f.store.clone().with_native_writing_context(missing),
            record.clone(),
        );
        let failed = unavailable
            .authorize_and_execute(search.clone())
            .await
            .unwrap();
        assert!(failed.is_error);
        assert!(!failed.content.contains("Frozen manuscript"));
        let restored = tools.authorize_and_execute(search.clone()).await.unwrap();
        assert!(!restored.is_error, "{}", restored.content);
        assert_eq!(proofs(), next_proofs);
        f.store
            .connection()
            .unwrap()
            .execute(
                "update bibliographic_items set trashed_at='now' where id='structured-paper'",
                [],
            )
            .unwrap();
        let revoked = tools.authorize_and_execute(search).await.unwrap();
        assert!(revoked.is_error);
        assert!(!revoked.content.contains("Current portrait"));
        assert_eq!(proofs(), next_proofs);
        assert!(f.store.connection().unwrap().is_autocommit());
        assert_eq!(
            std::fs::read_to_string(
                f.store
                    .manuscript_directory()
                    .join(f.manuscript.to_string())
                    .join("source/main.tex")
            )
            .unwrap(),
            "Frozen manuscript"
        );
        drop((tools, original, unavailable));
        native.service.shutdown().await.unwrap();
        f.store.shared_agent_connection().shutdown().unwrap();
    }
}
