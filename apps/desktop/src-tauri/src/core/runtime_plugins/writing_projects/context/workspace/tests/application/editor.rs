use super::*;

#[tokio::test]
#[ignore = "requires actual independent writing package; C04/C05 mixed editor and disk application"]
async fn actual_package_application_keeps_live_editor_dispatch_and_shared_file_save() {
    let (f, binding, _, _, _) = applied_editor().await;
    binding.service.shutdown().await.unwrap();
    f.store.shared_agent_connection().shutdown().unwrap();
}

pub(super) async fn applied_editor() -> (Fixture, Binding, Scope, Uuid, Uuid) {
    let (f, binding, scope, session, revision) = partial_editor().await;
    let complete = serde_json::to_value(apply(&f.store, &scope, "new.tex").await.unwrap()).unwrap();
    assert_eq!(complete["status"], "applied");
    (f, binding, scope, session, revision)
}

pub(super) async fn partial_editor() -> (Fixture, Binding, Scope, Uuid, Uuid) {
    use crate::agent::writing_workspace_commands::Buffer;
    use sha2::{Digest, Sha256};
    let (f, binding, scope) = proposal().await;
    let session = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let disk_sha = format!("{:x}", Sha256::digest(b"Frozen manuscript"));
    commands::execute(
        &f.store,
        Request::Begin {
            scope: scope.clone(),
            buffers: vec![Buffer {
                path: "main.tex".into(),
                expected_disk_sha256: disk_sha.clone(),
                session_id: session,
                revision,
                text: "Frozen manuscript".into(),
            }],
        },
    )
    .await
    .unwrap();
    let dispatched = commands::execute(
        &f.store,
        Request::PrepareEditor {
            scope: scope.clone(),
            current: commands::Observation {
                path: "main.tex".into(),
                session_id: session,
                revision,
                text: "Frozen manuscript".into(),
            },
        },
    )
    .await
    .unwrap();
    let commands::Response::Edit { expected, text, .. } = dispatched else {
        panic!("original live editor edit")
    };
    assert_eq!(expected.session_id, session);
    assert_eq!(expected.revision, revision);
    assert_eq!(expected.text, "Frozen manuscript");
    assert_eq!(text, "Revised manuscript");
    // The host's actual editor port supplies its observed revision and text.
    let result_revision = Uuid::new_v4();
    commands::execute(
        &f.store,
        Request::ConfirmEditor {
            scope: scope.clone(),
            actual: commands::Observation {
                path: "main.tex".into(),
                session_id: session,
                revision: result_revision,
                text: text.clone(),
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(source_root(&f).join("main.tex")).unwrap(),
        b"Frozen manuscript"
    );
    crate::runtime_plugins::writing_projects::files::operate_files(
        f.store.clone(),
        binding.service.clone(),
        binding.catalog.clone(),
        binding.package_key.clone(),
        f.manuscript.to_string(),
        crate::runtime_plugins::writing_projects::files::FileRequest::Save {
            request: research_workbench_native_sdk::writing_project::SaveFileRequest {
                manuscript_id: f.manuscript.to_string(),
                relative_path: "main.tex".into(),
                content: text,
                expected_sha256: Some(disk_sha),
            },
        },
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read(source_root(&f).join("main.tex")).unwrap(),
        b"Revised manuscript"
    );
    apply(&f.store, &scope, "remove.tex").await.unwrap();
    (f, binding, scope, session, result_revision)
}
