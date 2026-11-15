//! The two original snapshot reads used by an explicitly frozen source.
use super::*;
use crate::{
    ai::paper_reading_contracts::SnapshotEvidenceRow,
    data_interface::plugin_resources::PluginDataError,
};
use rusqlite::{OptionalExtension, params};

#[derive(Debug)]
pub(crate) enum SourceReadFailure {
    Static,
    Native(Box<PluginDataError>),
}

/// Connection and row conversion stay inside Core, including legacy transactions.
pub(crate) trait ExplicitReadingEvidence: Send + Sync {
    fn snapshot_hash(
        &self,
        db: &Connection,
        snapshot: &str,
        paper: &str,
    ) -> Result<Option<String>, SourceReadFailure>;
    fn snapshot_evidence_rows(
        &self,
        db: &Connection,
        snapshot: &str,
    ) -> Result<Vec<SnapshotEvidenceRow>, SourceReadFailure>;
}

impl ExplicitReadingEvidence for PaperReadingSource {
    fn snapshot_hash(
        &self,
        db: &Connection,
        snapshot: &str,
        paper: &str,
    ) -> Result<Option<String>, SourceReadFailure> {
        (self.snapshot_hash)(db, snapshot, paper).map_err(|_| SourceReadFailure::Static)
    }
    fn snapshot_evidence_rows(
        &self,
        db: &Connection,
        snapshot: &str,
    ) -> Result<Vec<SnapshotEvidenceRow>, SourceReadFailure> {
        (self.snapshot_evidence_rows)(db, snapshot).map_err(|_| SourceReadFailure::Static)
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub(crate) struct ExplicitEvidenceError {
    message: String,
    pub(crate) data: Option<Box<PluginDataError>>,
    non_data_read: Option<FailureCause>,
}
impl ExplicitEvidenceError {
    pub(super) fn read(message: &str, cause: SourceReadFailure) -> Self {
        let unavailable = matches!(&cause, SourceReadFailure::Static);
        Self {
            message: message.into(),
            data: match cause {
                SourceReadFailure::Static => None,
                SourceReadFailure::Native(error) => Some(error),
            },
            non_data_read: unavailable.then_some(FailureCause::SourceReadUnavailable),
        }
    }
}
impl ExplicitEvidenceError {
    fn sql(message: impl Into<String>, cause: &rusqlite::Error) -> Self {
        Self {
            message: message.into(),
            data: None,
            non_data_read: Some(FailureCause::SourceDatabase(Box::new(
                crate::runtime_plugins::native::SafeDatabaseError::sql("source-read", cause),
            ))),
        }
    }
    pub(crate) fn into_failure(self) -> Failure<String> {
        let error = self.to_string();
        let cause = match self.data {
            Some(data) => FailureCause::SourceData(data),
            None => self.non_data_read.unwrap_or(FailureCause::Original),
        };
        Failure { error, cause }
    }
}
impl From<String> for ExplicitEvidenceError {
    fn from(message: String) -> Self {
        Self {
            message,
            data: None,
            non_data_read: None,
        }
    }
}
impl From<&str> for ExplicitEvidenceError {
    fn from(message: &str) -> Self {
        message.to_owned().into()
    }
}

pub(crate) fn explicit_evidence(
    reading: &dyn ExplicitReadingEvidence,
    db: &Connection,
    paper: &str,
    source: &str,
) -> Result<AiEvidenceBundle, ExplicitEvidenceError> {
    explicit_evidence_for(reading, db, paper, source)
}

pub(super) fn explicit_evidence_for<R: ExplicitReadingEvidence + ?Sized>(
    reading: &R,
    db: &Connection,
    paper: &str,
    source: &str,
) -> Result<AiEvidenceBundle, ExplicitEvidenceError> {
    if let Some(anchor) = source.strip_prefix("anchor:") {
        let row=db.query_row("select s.attachment_id,s.content_sha256,s.page_index,s.quote_exact,s.quote_prefix,s.quote_suffix,coalesce(a.content_sha256,'') from source_anchors s left join attachments a on a.id=s.attachment_id where s.id=?1 and s.item_id=?2",params![anchor,paper],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?,r.get::<_,String>(6)?))).optional().map_err(|error| ExplicitEvidenceError::sql("无法读取保存的原文锚点", &error))?.ok_or("原文锚点不存在或属于其他文献")?;
        let mut source_item = item(paper, anchor.into(), row.0, row.1.clone(), row.2, row.3);
        source_item.anchor_id = anchor.into();
        source_item.quote_prefix = row.4;
        source_item.quote_suffix = row.5;
        source_item.current_anchor_status = anchor_status(&row.6, &row.1).into();
        return Ok(AiEvidenceBundle {
            id: format!("anchor:{anchor}"),
            run_id: String::new(),
            scope_hash: String::new(),
            retrieval_config_version: "structured-anchor-v1".into(),
            validation_status: "valid".into(),
            created_at: String::new(),
            items: vec![source_item],
        });
    }
    let bundle_id = db
        .query_row(
            "select id from ai_evidence_bundles where id=?1 or run_id=?1",
            [source],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| ExplicitEvidenceError::sql("无法读取冻结来源", &error))?;
    if let Some(id) = bundle_id {
        let bundle = super::super::evidence_repository::load_bundle(db, &id).map_err(|error| {
            let message = error.to_string();
            match error {
                AiError::Database(cause) => ExplicitEvidenceError::sql(message, &cause),
                _ => ExplicitEvidenceError::from(message),
            }
        })?;
        if bundle.items.iter().any(|item| item.paper_id != paper) {
            return Err("冻结来源属于其他文献".into());
        }
        return Ok(bundle);
    }
    let belongs = reading
        .snapshot_hash(db, source, paper)
        .map_err(|cause| ExplicitEvidenceError::read("无法读取冻结来源", cause))?
        .is_some();
    if !belongs {
        return Err("冻结来源不存在或属于其他文献".into());
    }
    let rows = reading
        .snapshot_evidence_rows(db, source)
        .map_err(|cause| ExplicitEvidenceError::read("无法读取冻结原文", cause))?;
    let items = rows
        .into_iter()
        .map(|(id, attachment, sha, page, text, current_sha)| {
            let status = anchor_status(&current_sha, &sha);
            let mut source = item(paper, id, attachment, sha, page, text);
            source.current_anchor_status = status.into();
            source
        })
        .collect();
    Ok(AiEvidenceBundle {
        id: source.into(),
        run_id: String::new(),
        scope_hash: String::new(),
        retrieval_config_version: "structured-resource-v1".into(),
        validation_status: "valid".into(),
        created_at: String::new(),
        items,
    })
}
