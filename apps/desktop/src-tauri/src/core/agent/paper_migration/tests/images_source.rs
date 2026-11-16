pub(super) const CANONICAL_SNAPSHOT: &str = "canonical-picture-fixture";

pub(super) fn reading_response_source() -> crate::ai::paper_reading_source::PaperReadingSource {
    use crate::ai::paper_reading_contracts::PaperContextError;
    use rusqlite::OptionalExtension;
    crate::ai::paper_reading_source::PaperReadingSource {
        attachment_path: |_, _, _| panic!("unused fixture attachment lookup"),
        prepare: |_, _, _| panic!("unused fixture preparation"),
        snapshot_ready: |_| panic!("unused fixture readiness"),
        build: |_, _, _| panic!("unused fixture build"),
        persist: |_, _, _, _| panic!("unused fixture publication"),
        load: |_, _| panic!("unused fixture load"),
        recanonicalize: |db, id| {
            let exists: bool = db.query_row(
                "select exists(select 1 from paper_context_snapshots where id=?1)",
                [id],
                |row| row.get(0),
            )?;
            assert!(exists);
            Ok(db.query_row(
                "select snapshot_id,context_hash from fixture_reading_response",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?)
        },
        preview: |db, request| {
            let encoded: String = db
                .query_row(
                    "select preview_cbor from fixture_reading_response",
                    [],
                    |row| row.get(0),
                )
                .map_err(|_| PaperContextError::Corrupt)?;
            let preview: crate::ai::paper_reading_contracts::PaperContextPreview =
                research_workbench_native_sdk::paper_context_wire::decode_payload(&encoded)
                    .map_err(|_| PaperContextError::Corrupt)?;
            assert_eq!(request.snapshot_id, preview.snapshot_id);
            assert_eq!(request.expected_context_hash, preview.context_hash);
            Ok(preview)
        },
        latest: |_, _| panic!("unused fixture latest lookup"),
        snapshot_hash: |db, snapshot, paper| {
            db.query_row(
                "select context_hash from paper_context_snapshots where id=?1 and paper_id=?2",
                [snapshot, paper],
                |row| row.get(0),
            )
            .optional()
            .map_err(Into::into)
        },
        validation_latest_snapshot: |_, _| panic!("unused fixture validation lookup"),
        snapshot_evidence_rows: |_, _| panic!("unused fixture evidence lookup"),
        item_geometry: |_, _, _| panic!("unused fixture geometry lookup"),
        frozen_evidence_belongs: |_, _, _| panic!("unused fixture evidence ownership"),
        save_images: |_, _| panic!("unused fixture image staging"),
        validate_images: |_, _| panic!("unused fixture image validation"),
    }
}
