//! Both independently built domain hosts share the original Core source evaluator.
use super::*;
use crate::{
    agent::{context_session::sources::Resource, repository::submissions},
    runtime_plugins::{native::NativePreviewService, writing_projects::context::Binding},
};
use sha2::{Digest, Sha256};
use uuid::Uuid;
mod evidence;

fn mixed_fixture(manuscript_first: bool) -> (Fixture, Binding, Uuid) {
    let (catalog, key) = actual_package();
    let writing_key = catalog
        .discover()
        .unwrap()
        .packages
        .into_iter()
        .find(|package| package.manifest["id"] == "writing_projects")
        .expect("the actual collection must contain the independently built writing package")
        .package_key;
    let writing = Binding {
        service: Arc::new(NativePreviewService::new(catalog.clone())),
        catalog: catalog.clone(),
        package_key: writing_key,
    };
    let mut fixture = Fixture::new();
    fixture.bind_actual_package(catalog, key);
    fixture.store = fixture
        .store
        .clone()
        .with_native_writing_context(writing.clone());
    assert!(fixture.store.writing_resource_source().is_none());
    let manuscript = Uuid::new_v4();
    let body = "Manuscript Ω";
    let file = Resource::ManuscriptFile {
        resource_id: manuscript,
        path: "main.tex".into(),
    };
    {
        let mut db = fixture.store.connection().unwrap();
        db.execute("insert into research_projects(id,title,summary,created_at) values('mixed-project','Mixed project','','now')", []).unwrap();
        db.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values(?1,'mixed-project','Mixed manuscript','main.tex','now','now')", [manuscript.to_string()]).unwrap();
        db.execute("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values(?1,'main.tex','tex',?2,?3,'now')", rusqlite::params![manuscript.to_string(), body.len(), format!("{:x}", Sha256::digest(body))]).unwrap();
        let mut resources = vec![
            Resource::Note {
                resource_id: Uuid::parse_str(&fixture.note).unwrap(),
            },
            Resource::PaperMetadata {
                resource_id: "paper".into(),
            },
        ];
        if manuscript_first {
            resources.insert(0, file);
        } else {
            resources.push(file);
        }
        let tx = db.transaction().unwrap();
        let mut input = fixture.record.input.clone();
        input.context =
            crate::agent::context_session::resource_grants::grants(&tx, &resources, None).unwrap();
        fixture.record =
            submissions::insert_in(&tx, fixture.record.conversation_id, Uuid::new_v4(), input)
                .unwrap();
        tx.commit().unwrap();
    }
    let directory = fixture
        .store
        .manuscript_directory()
        .join(manuscript.to_string())
        .join("source");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("main.tex"), body).unwrap();
    (fixture, writing, manuscript)
}

async fn prepare_dynamic_scope(fixture: &mut Fixture, writing: &Binding) {
    use crate::{
        agent::context_session::{PrepareRequest, sources::Access},
        data_interface::{ContextPrepareIntent, ContextPrepareSource},
    };
    let resources = fixture
        .record
        .input
        .context
        .iter()
        .map(|source| {
            match crate::agent::context_session::resource_grants::resource(source).unwrap() {
                Resource::ManuscriptFile { resource_id, .. } => Resource::Manuscript {
                    resource_id: resource_id.to_string(),
                    parts: vec![],
                    new_member_access: Some(Access::ReadOnly),
                    group_access: Some(Access::ReadOnly),
                },
                resource => resource,
            }
        })
        .collect();
    let operation = Uuid::new_v4();
    writing
        .prepare_independent(
            fixture.store.shared_agent_connection(),
            ContextPrepareIntent {
                request: PrepareRequest {
                    conversation_id: fixture.record.conversation_id,
                    request_id: operation,
                    text: fixture.record.input.text.clone(),
                    resources,
                    user_input: vec![],
                    retrieval: None,
                    model: None,
                    queue: None,
                },
                expected_epoch: None,
            },
            ContextPrepareSource {
                store: fixture.store.clone(),
                model: (fixture.record.input.model.clone(), None, None),
            },
        )
        .await
        .unwrap();
    fixture.record = submissions::read(
        &fixture.store.connection().unwrap(),
        fixture.record.conversation_id,
        operation,
    )
    .unwrap()
    .unwrap();
}

#[tokio::test]
#[ignore = "requires actual independent literature and writing package collection"]
async fn actual_two_packages_search_metadata_notes_and_manuscripts_in_original_order() {
    for dynamic in [false, true] {
        for manuscript_first in [false, true] {
            let (mut fixture, writing, manuscript) = mixed_fixture(manuscript_first);
            if dynamic {
                prepare_dynamic_scope(&mut fixture, &writing).await;
            }
            let request = fixture.search();
            let first = fixture.execute(request.clone()).await;
            assert!(!first.is_error, "{}", first.content);
            let first: Value = serde_json::from_str(&first.content).unwrap();
            let mut actual: Vec<_> = first["matches"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["sourceId"].as_str().unwrap())
                .collect();
            actual.dedup();
            let file_id = crate::agent::context_session::resource_grants::grants(
                &fixture.store.connection().unwrap(),
                &[Resource::ManuscriptFile {
                    resource_id: manuscript,
                    path: "main.tex".into(),
                }],
                None,
            )
            .unwrap()
            .remove(0)
            .source_id;
            let expected: Vec<_> = fixture
                .record
                .input
                .context
                .iter()
                .map(|source| {
                    if matches!(
                        crate::agent::context_session::resource_grants::resource(source),
                        Some(Resource::Manuscript { .. })
                    ) {
                        file_id.as_str()
                    } else {
                        source.source_id.as_str()
                    }
                })
                .collect();
            assert_eq!(actual, expected);
            assert_eq!(fixture.proofs(), 1);
            let metadata_body = expected_metadata(&fixture.store.connection().unwrap(), "paper");
            let expected_metadata =
                source_tools::evaluate(&metadata_record(&fixture.record, metadata_body), &request)
                    .unwrap();
            let metadata_id = &expected_metadata["matches"][0]["sourceId"];
            let actual_metadata: Vec<_> = first["matches"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["sourceId"] == *metadata_id)
                .collect();
            let expected_matches = expected_metadata["matches"].as_array().unwrap();
            assert_eq!(actual_metadata.len(), expected_matches.len());
            for (actual_metadata, expected) in actual_metadata.into_iter().zip(expected_matches) {
                for field in [
                    "sourceId",
                    "kind",
                    "resourceId",
                    "startChar",
                    "endChar",
                    "snippetStartChar",
                    "snippet",
                ] {
                    assert_eq!(actual_metadata[field], expected[field], "{field}");
                }
            }
            let repeated = fixture.execute(request.clone()).await;
            assert!(!repeated.is_error, "{}", repeated.content);
            assert_eq!(
                serde_json::from_str::<Value>(&repeated.content).unwrap(),
                first
            );
            assert_eq!(fixture.proofs(), 1);
            let directory = fixture
                .store
                .manuscript_directory()
                .join(manuscript.to_string())
                .join("source");
            let updated = "Updated manuscript Ω";
            fs::write(directory.join("main.tex"), updated).unwrap();
            fixture.store.connection().unwrap().execute(
            "update manuscript_files set sha256=?1,byte_size=?2 where manuscript_id=?3 and relative_path='main.tex'",
            rusqlite::params![format!("{:x}", Sha256::digest(updated)), updated.len(), manuscript.to_string()],
        ).unwrap();
            let current = fixture.execute(request.clone()).await;
            assert!(!current.is_error, "{}", current.content);
            assert!(current.content.contains(updated));
            let mut missing = writing.clone();
            missing.package_key = "0".repeat(64);
            fixture.store = fixture.store.clone().with_native_writing_context(missing);
            let rejected = fixture.execute(request.clone()).await;
            assert!(rejected.is_error);
            let diagnostic: Value = serde_json::from_str(&rejected.content).unwrap();
            if dynamic {
                // Dynamic members require the original directory admission before body reads.
                let message = diagnostic["error"].as_str().unwrap();
                assert!(message.starts_with("plugin_host_package_unavailable:"));
                assert!(message.contains(&format!("packageKey={}", "0".repeat(64))));
                assert!(message.contains("phase=admission"));
                assert!(message.contains("cause=plugin_host_package_unavailable"));
            } else {
                assert_eq!(diagnostic["error"], "writing_source_read_unavailable");
                assert_eq!(
                    diagnostic["requestId"],
                    fixture.record.request_id.to_string()
                );
                assert_eq!(diagnostic["turnId"], fixture.record.turn_id.to_string());
                assert_eq!(diagnostic["callId"], request.call_id);
                assert!(diagnostic["writingSource"].is_object());
            }
            assert!(!rejected.content.contains(updated));
            fixture.store = fixture
                .store
                .clone()
                .with_native_writing_context(writing.clone());
            let restored = fixture.execute(request.clone()).await;
            assert!(!restored.is_error, "{}", restored.content);
            assert!(restored.content.contains(updated));
            fixture
                .store
                .connection()
                .unwrap()
                .execute(
                    "update note_pages set status='trashed' where id=?1",
                    [&fixture.note],
                )
                .unwrap();
            let revoked = fixture.execute(request).await;
            assert!(revoked.is_error);
            assert!(!revoked.content.contains(updated));
            if !manuscript_first && !dynamic {
                let mut missing = writing.clone();
                missing.package_key = "0".repeat(64);
                fixture.store = fixture.store.clone().with_native_writing_context(missing);
                let earlier_error = fixture.execute(fixture.search()).await;
                let value: Value = serde_json::from_str(&earlier_error.content).unwrap();
                assert!(earlier_error.is_error);
                assert_eq!(value["error"], "selected source unavailable or changed");
                assert!(value.get("writingSource").is_none());
            }
            assert!(fixture.store.connection().unwrap().is_autocommit());
            assert_eq!(fixture.proofs(), 1);
            writing.service.shutdown().await.unwrap();
            fixture.service.shutdown().await.unwrap();
            fixture.store.shared_agent_connection().shutdown().unwrap();
        }
    }
}

#[tokio::test]
#[ignore = "requires actual independent literature and writing package collection"]
async fn actual_two_packages_keep_structured_reads_in_metadata_and_manuscript_order() {
    use crate::agent::context_session::{
        resource_grants, sources::Access, structured::TEMPLATE_CATALOG_ID,
    };
    for manuscript_first in [false, true] {
        let (mut fixture, writing, _) = mixed_fixture(manuscript_first);
        {
            let mut resources: Vec<_> = fixture
                .record
                .input
                .context
                .iter()
                .map(|source| resource_grants::resource(source).unwrap())
                .collect();
            resources.insert(
                1,
                Resource::PaperPortrait {
                    resource_id: "paper".into(),
                },
            );
            resources.push(Resource::TemplateCatalog {
                resource_id: TEMPLATE_CATALOG_ID.into(),
                access: Some(Access::ReadOnly),
            });
            let mut db = fixture.store.connection().unwrap();
            let tx = db.transaction().unwrap();
            let mut input = fixture.record.input.clone();
            input.context = resource_grants::grants(&tx, &resources, None).unwrap();
            fixture.record =
                submissions::insert_in(&tx, fixture.record.conversation_id, Uuid::new_v4(), input)
                    .unwrap();
            tx.commit().unwrap();
        }
        let output = fixture.execute(fixture.search()).await;
        assert!(!output.is_error, "{}", output.content);
        let value: Value = serde_json::from_str(&output.content).unwrap();
        let mut actual: Vec<_> = value["matches"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["sourceId"].as_str().unwrap())
            .collect();
        actual.dedup();
        let expected: Vec<_> = fixture
            .record
            .input
            .context
            .iter()
            .filter_map(|source| match resource_grants::resource(source).unwrap() {
                Resource::PaperPortrait { .. } => Some(
                    resource_grants::grants(
                        &fixture.store.connection().unwrap(),
                        &[Resource::StructuredObject {
                            resource_id: fixture.portrait.clone(),
                            access: Some(Access::ReadOnly),
                        }],
                        None,
                    )
                    .unwrap()
                    .remove(0)
                    .source_id,
                ),
                Resource::TemplateCatalog { .. } => None,
                _ => Some(source.source_id.clone()),
            })
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(
            fixture.proofs(),
            2,
            "Core Note and original portrait journal"
        );
        let repeated = fixture.execute(fixture.search()).await;
        assert!(!repeated.is_error, "{}", repeated.content);
        assert_eq!(
            serde_json::from_str::<Value>(&repeated.content).unwrap(),
            value
        );
        assert_eq!(fixture.proofs(), 2);
        fixture
            .store
            .connection()
            .unwrap()
            .execute(
                "update structured_resources set content_json=?1,revision=revision+1 where id=?2",
                rusqlite::params![
                    json!({"description":"Current portrait Ω"}).to_string(),
                    fixture.portrait
                ],
            )
            .unwrap();
        let current = fixture.execute(fixture.search()).await;
        assert!(!current.is_error, "{}", current.content);
        assert!(current.content.contains("Current portrait Ω"));
        assert_eq!(fixture.proofs(), 2);
        assert!(fixture.store.connection().unwrap().is_autocommit());
        writing.service.shutdown().await.unwrap();
        fixture.service.shutdown().await.unwrap();
        fixture.store.shared_agent_connection().shutdown().unwrap();
    }
}
