use super::super::apply_disk::deleted_chat::{delete_chat, full_fixture};
use super::*;

#[tokio::test]
async fn deleted_origin_preserves_editor_checkpoints_and_checkpointed_partial_undo() {
    let f = full_fixture();
    let s = undo_checkpoint::partial(&f).await;
    undo_checkpoint::persist(&f, &s.checkpoints);
    let frozen = s
        .service
        .read_proposal_undo(s.scope, s.undo)
        .await
        .unwrap()
        .unwrap();
    let read_checkpoints = || {
        let db = f.store.connection().unwrap();
        let mut query = db.prepare("select relative_path,revision,checkpoint_json from writing_document_checkpoints order by relative_path").unwrap();
        query
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let before = read_checkpoints();
    delete_chat(&f).await;
    assert_eq!(read_checkpoints(), before);
    assert_eq!(
        std::fs::read(
            f.store
                .manuscript_directory()
                .join(f.manuscript.to_string())
                .join("source/main.tex")
        )
        .unwrap(),
        b"Frozen manuscript"
    );
    let service = WritingSubmissionService::from_app_store(&f.store);
    let resumed = service
        .resume_proposal_undo_with_checkpoints(
            s.scope,
            s.undo,
            undo_checkpoint::copy_request(&s.request),
            s.proofs,
        )
        .await
        .unwrap();
    assert_eq!(resumed.changes, frozen.changes);
    assert_eq!(resumed.application.files[0], frozen.application.files[0]);
    assert_eq!(resumed.application.status, ApplicationStatus::Applying);
    assert_eq!(read_checkpoints(), before);
}
