use super::*;
use crate::{
    ai::structured_resource_validation::Flow,
    data_interface::plugin_resources::write_in,
    hbs_storage::boundary::{BoundaryError, confirm_with, settle_with},
};
use std::convert::Infallible;
impl PaperContextResources {
    pub(crate) fn apply_snapshot(
        &self,
        db: &mut Connection,
        paper: &str,
        projection: &SnapshotProjectionV1,
        captured: &SnapshotCapture,
        existing_digest: Option<&str>,
    ) -> ContextResult<PaperContextSnapshot> {
        if self.capture_snapshot(db, projection)? != *captured {
            return Err(failure::changed());
        }
        let mut repaired = false;
        if let Some(existing) = &captured.existing {
            if existing_digest.is_none() {
                return Err(failure::invalid("论文上下文快照缺少本次校验结果。"));
            }
            if existing_digest == Some(existing.context_hash.as_str()) {
                return Ok(existing.clone());
            }
            let protected = integrity::snapshot_is_protected(db, &existing.id).map_err(
                |error| match error {
                    integrity::ProtectionError::Database(error) => {
                        failure::sql(error, "source-read")
                    }
                    integrity::ProtectionError::Managed(error) => {
                        failure::ai(error.into_ai_error())
                    }
                },
            )?;
            if protected {
                return Err(failure::invalid(
                    "已引用的论文快照校验失败，请重新解析当前 PDF 后重试",
                ));
            }
            self.transaction(db, |tx| {
                for resource in [
                    R::SnapshotRepairDeleteItems,
                    R::SnapshotRepairDeleteAttachments,
                    R::SnapshotRepairDeleteHeader,
                ] {
                    self.write(tx, resource, vec![text(&existing.id)])?;
                }
                Ok(())
            })?;
            repaired = true;
        }
        self.transaction(db, |tx| {
            self.write(
                tx,
                R::SnapshotPersistInsertHeader,
                vec![
                    text(&projection.snapshot_id),
                    text(paper),
                    text(&projection.context_hash),
                    V::Integer(projection.context_schema_version),
                ],
            )?;
            for attachment in &projection.attachments {
                self.write(
                    tx,
                    R::SnapshotPersistInsertAttachment,
                    vec![
                        text(&projection.snapshot_id),
                        V::Integer(attachment.ordinal),
                        text(&attachment.attachment_id),
                        text(&attachment.content_sha256),
                        text(&attachment.parse_run_id),
                    ],
                )?;
            }
            for item in &projection.items {
                let regions = serde_json::to_string(&item.regions)
                    .map_err(|_| failure::original(AiError::TemporaryUnavailable))?;
                self.write(
                    tx,
                    R::SnapshotPersistInsertItem,
                    vec![
                        text(&projection.snapshot_id),
                        text(&item.id),
                        text(&item.attachment_id),
                        text(&item.content_sha256),
                        text(&item.parse_run_id),
                        text(&item.raw_ref),
                        text(&item.kind),
                        optional(item.label.as_deref()),
                        item.page_index
                            .map_or(V::Null, |page| V::Integer(i64::from(page))),
                        V::Text(regions),
                        item.charspan.as_ref().map_or(V::Null, |value| {
                            V::Text(
                                serde_json::to_string(value).unwrap_or_else(|_| "[]".to_owned()),
                            )
                        }),
                        optional(item.source_text_hash.as_deref()),
                        optional(item.text.as_deref()),
                        item.table_json
                            .as_ref()
                            .map_or(V::Null, |value| V::Text(value.to_string())),
                        optional(item.image_content_sha256.as_deref()),
                    ],
                )?;
            }
            Ok(())
        })
        .map_err(|mut error| {
            if repaired && error.outcome != HbsOutcome::Unknown {
                error.outcome = HbsOutcome::Committed;
            }
            error
        })?;
        self.load_snapshot(db, &projection.snapshot_id)
            .map_err(|mut error| {
                error.phase = "postcommit-read";
                error.outcome = HbsOutcome::Committed;
                error
            })
    }
    fn write(&self, tx: &rusqlite::Transaction<'_>, id: R, args: Vec<V>) -> ContextResult<()> {
        write_in(tx, self.resource(id), &self.call(id, args))
            .map(|_| ())
            .map_err(|error| {
                let mut error = failure::data(error);
                error.phase = "write";
                error
            })
    }
    fn transaction(
        &self,
        db: &mut Connection,
        operation: impl FnOnce(&rusqlite::Transaction<'_>) -> ContextResult<()>,
    ) -> ContextResult<()> {
        let tx = db
            .transaction()
            .map_err(|error| failure::sql(error, "begin"))?;
        let result = operation(&tx).map_err(|error| Flow::<_, Infallible>::Fail(error.failure));
        let settled = settle_with(tx, result, "write", true, true, AiError::Database, |_| {
            AiError::TemporaryUnavailable
        });
        match confirm_with(db, settled, true, |_| AiError::TemporaryUnavailable) {
            Ok(()) => Ok(()),
            Err(BoundaryError::Failure(error)) => Err(Box::new(error)),
            Err(BoundaryError::Pause(_)) => unreachable!("fixed snapshot writes never suspend"),
        }
    }
}
fn optional(value: Option<&str>) -> V {
    value.map_or(V::Null, text)
}
