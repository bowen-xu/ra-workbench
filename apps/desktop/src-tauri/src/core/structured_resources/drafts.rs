use super::{
    Error, PatchDraft, Receipt, ValidationIssue, WriteDraft, WriteResult, content, decode, encode,
    get, hash, repository, validation,
};
use crate::{
    ai::structured_resource_validation::{FailureCause, Flow, StructuredPolicy},
    hbs_storage::boundary::{BoundaryError, HbsOutcome, HbsSources, StaticHbsSources},
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;
#[path = "drafts/recovery.rs"]
mod recovery;
#[path = "drafts/transaction.rs"]
pub(super) mod transaction;
type WriteBoundary<S> = Result<
    WriteResult,
    BoundaryError<<<S as HbsSources>::Policy as StructuredPolicy>::Pause, Error>,
>;

pub(crate) fn write(
    policy: crate::ai::generation_policy::GenerationPolicySource,
    reading_source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &mut Connection,
    input: WriteDraft,
) -> Result<WriteResult, Error> {
    execute(policy, reading_source, db, input, None)
}

pub(crate) fn write_with<S: HbsSources>(
    sources: &mut S,
    db: &mut Connection,
    input: WriteDraft,
) -> WriteBoundary<S> {
    execute_with(sources, db, input, None)
}
fn execute(
    policy: crate::ai::generation_policy::GenerationPolicySource,
    reading_source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &mut Connection,
    input: WriteDraft,
    identity: Option<String>,
) -> Result<WriteResult, Error> {
    execute_with(
        &mut StaticHbsSources::new(Some(policy), Some(reading_source)),
        db,
        input,
        identity,
    )
    .map_err(BoundaryError::into_static)
}
fn execute_with<S: HbsSources>(
    sources: &mut S,
    db: &mut Connection,
    mut input: WriteDraft,
    identity: Option<String>,
) -> WriteBoundary<S> {
    let explicit = sources.explicit_settlement();
    let fingerprint = identity.unwrap_or(hash(
        &encode(&input).map_err(|error| transaction::before(error, "receipt"))?,
    ));
    if let Some(result) = operation(db, &input.request_id, &fingerprint)
        .map_err(|error| transaction::before(error, "receipt"))?
    {
        return Ok(result);
    }
    let staged = if explicit {
        super::write_ahead::stage_with(db, &input, &fingerprint, true)?
    } else {
        super::write_ahead::stage(db, &input, &fingerprint)
            .map_err(|error| transaction::before(error, "write"))?
    };
    let resource =
        get(db, &input.resource_id).map_err(|error| transaction::before(error, "read"))?;
    let source: Option<String> = db
        .query_row(
            "select source_snapshot_id from structured_resource_drafts where id=?1 and revision=?2",
            params![staged.id, staged.revision],
            |r| r.get(0),
        )
        .map_err(|error| transaction::before(error, "source-read"))?;
    let checked = validation::validate_write_with(
        sources,
        db,
        &resource,
        &input.content,
        source.as_deref(),
        input.editor_kind == "human",
    );
    input.expected_draft_revision = Some(staged.revision);
    let result = execute_inner(
        db,
        input.clone(),
        Some(fingerprint.clone()),
        checked,
        explicit,
    );
    match result {
        Err(BoundaryError::Failure(failure))
            if matches!(failure.failure.error, Error::Database(_))
                && (!explicit
                    || (failure.outcome != HbsOutcome::Unknown && db.is_autocommit())) =>
        {
            let Error::Database(error) = failure.failure.error else {
                unreachable!()
            };
            recovery::recover(db, input, &fingerprint, &staged, error, explicit)
        }
        result => result,
    }
}
fn execute_inner<P>(
    db: &mut Connection,
    input: WriteDraft,
    identity: Option<String>,
    checked: Result<Value, Flow<Vec<ValidationIssue>, P>>,
    explicit: bool,
) -> Result<WriteResult, BoundaryError<P, Error>> {
    if input.request_id.trim().is_empty()
        || input.request_id.len() > 200
        || input.content.len() > 8 * 1024 * 1024
        || !["human", "agent", "migration"].contains(&input.editor_kind.as_str())
    {
        return Err(transaction::before(
            Error::Invalid("写入请求身份、来源或容量无效".into()),
            "execution",
        ));
    }
    let request_hash = identity.unwrap_or(hash(
        &encode(&input).map_err(|error| transaction::before(error, "receipt"))?,
    ));
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| transaction::before(error, "begin"))?;
    let mut phase = "read";
    let result = (|| -> Result<(WriteResult, bool), Flow<Error, P>> {
        if let Some(result) = operation(&tx, &input.request_id, &request_hash)? {
            return Ok((result, false));
        }
        if let Some(receipt) = receipt(&tx, &input.request_id, &request_hash)? {
            let resource = get(&tx, &receipt.resource_id)?;
            return Ok((
                WriteResult {
                    status: "applied".into(),
                    resource,
                    draft: None,
                    errors: vec![],
                    receipt: Some(receipt),
                    validation_origin: None,
                },
                false,
            ));
        }
        let resource = get(&tx, &input.resource_id)?;
        let old = repository::get_draft(&tx, &resource.id, input.conversation_id.as_deref())?;
        let mut draft = old.ok_or(Error::Conflict)?;
        if draft.content != input.content {
            return Err(Error::Conflict.into());
        }
        draft.errors.clear();
        let legacy_required = if let Some(paper) = resource.paper_id.as_deref() {
            match super::require_legacy_imported(&tx, paper) {
                Ok(()) => false,
                Err(Error::LegacyImportRequired) => true,
                Err(error) => return Err(error.into()),
            }
        } else {
            false
        };
        phase = "validation";
        let mut validation_origin = None;
        let (status, value) = if legacy_required {
            draft.errors = vec![ValidationIssue {
                path: "$".into(),
                code: "legacy_import_required".into(),
                message: Error::LegacyImportRequired.to_string(),
            }];
            ("invalid", None)
        } else if resource.revision != input.expected_revision {
            draft.errors = vec![ValidationIssue {
                path: "$".into(),
                code: "revision_conflict".into(),
                message: format!("正式内容已更新到版本{}；草稿已保留", resource.revision),
            }];
            ("conflict", None)
        } else {
            match checked {
                Ok(value) => ("applied", Some(value)),
                Err(Flow::Need(need)) => return Err(Flow::Need(need)),
                Err(Flow::Fail(failure)) => {
                    if explicit
                        && !matches!(
                            failure.cause,
                            FailureCause::Original | FailureCause::NativeRejection(_)
                        )
                    {
                        return Err(Flow::Fail(failure.map_error(|issues| {
                            Error::Invalid(
                                issues
                                    .iter()
                                    .map(|issue| issue.message.as_str())
                                    .collect::<Vec<_>>()
                                    .join("；"),
                            )
                        })));
                    }
                    if let FailureCause::NativeRejection(origin) = failure.cause {
                        validation_origin = Some(super::ValidationOrigin {
                            request_id: input.request_id.clone(),
                            package_key: origin.package_key,
                            instance_id: origin.instance_id,
                            native_request_id: origin.native_request_id,
                        });
                    }
                    draft.errors = failure.error;
                    ("invalid", None)
                }
            }
        };
        phase = "write";
        tx.execute(
            "update structured_resource_drafts set errors_json=?3 where id=?1 and revision=?2",
            params![draft.id, draft.revision, encode(&draft.errors)?],
        )?;
        if status != "applied" {
            let mut resource = resource;
            resource.draft = Some(draft.clone());
            let result = WriteResult {
                status: status.into(),
                errors: draft.errors.clone(),
                resource,
                draft: Some(draft),
                receipt: None,
                validation_origin,
            };
            save_operation(&tx, &input, &request_hash, &result)?;
            return Ok((result, true));
        }
        if let Some(paper) = resource.paper_id.as_deref() {
            super::legacy_guard::mark_no_legacy(&tx, paper)?;
        }
        let mut published = resource;
        let previous_content = published.content.clone();
        let previous_pending = published.pending_field_ids.clone();
        published.content = value;
        let changed = content::pending(published.content.as_ref(), previous_content.as_ref());
        let protected: Vec<String> = tx
            .query_row(
                "select protected_fields_json from structured_resources where id=?1",
                [&published.id],
                |r| r.get::<_, String>(0),
            )
            .map_err(Error::from)
            .and_then(|v| decode(&v))?;
        let mut protected = protected
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        protected.extend(
            published
                .reviewed_field_ids
                .iter()
                .filter(|id| changed.contains(id))
                .cloned(),
        );
        published
            .reviewed_field_ids
            .retain(|id| !changed.contains(id));
        published.revision += 1;
        if input.editor_kind == "human" {
            if previous_pending.is_empty() {
                published.review_baseline = published.content.clone();
            } else {
                let synchronized = changed
                    .iter()
                    .filter(|id| !previous_pending.contains(id))
                    .cloned()
                    .collect::<Vec<_>>();
                content::replace_fields(
                    &mut published.review_baseline,
                    published.content.as_ref(),
                    &synchronized,
                    true,
                );
            }
        }
        published.pending_field_ids = content::pending(
            published.content.as_ref(),
            published.review_baseline.as_ref(),
        );
        protected.retain(|id| published.pending_field_ids.contains(id));
        let source: Option<String> = tx.query_row(
            "select source_snapshot_id from structured_resource_drafts where id=?1",
            [&draft.id],
            |row| row.get(0),
        )?;
        repository::persist_revision(
            &tx,
            &published,
            &input.editor_kind,
            source.as_deref(),
            &protected.into_iter().collect::<Vec<_>>(),
        )?;

        if let Some(name) = published
            .content
            .as_ref()
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
        {
            tx.execute(
                "update structured_resources set title=?2 where id=?1",
                params![published.id, name],
            )?;
            published.title = name.into();
        }
        let receipt = Receipt {
            request_id: input.request_id.clone(),
            resource_id: published.id.clone(),
            revision: published.revision,
            content_hash: hash(&encode(&published.content)?),
        };
        phase = "receipt";
        save_receipt(
            &tx,
            &receipt,
            "write",
            &request_hash,
            input.conversation_id.as_deref(),
            input.submission_id.as_deref(),
        )?;
        tx.execute(
            "delete from structured_resource_drafts where id=?1 and revision=?2",
            params![draft.id, draft.revision],
        )?;
        let result = WriteResult {
            status: "applied".into(),
            resource: published,
            draft: None,
            errors: vec![],
            receipt: Some(receipt),
            validation_origin: None,
        };
        save_operation(&tx, &input, &request_hash, &result)?;
        Ok((result, true))
    })();
    let settled = transaction::settle(tx, result, phase, explicit);
    transaction::confirm(db, settled, explicit)
}

pub(crate) fn patch(
    policy: crate::ai::generation_policy::GenerationPolicySource,
    reading_source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &mut Connection,
    input: PatchDraft,
) -> Result<WriteResult, Error> {
    patch_with(
        &mut StaticHbsSources::new(Some(policy), Some(reading_source)),
        db,
        input,
    )
    .map_err(BoundaryError::into_static)
}

pub(crate) fn patch_with<S: HbsSources>(
    sources: &mut S,
    db: &mut Connection,
    input: PatchDraft,
) -> WriteBoundary<S> {
    let identity = hash(&encode(&input).map_err(|error| transaction::before(error, "receipt"))?);
    if let Some(result) = operation(db, &input.request_id, &identity)
        .map_err(|error| transaction::before(error, "receipt"))?
    {
        return Ok(result);
    }
    if let Some(receipt) = receipt(db, &input.request_id, &identity)
        .map_err(|error| transaction::before(error, "receipt"))?
    {
        return Ok(WriteResult {
            status: "applied".into(),
            resource: get(db, &receipt.resource_id)
                .map_err(|error| transaction::before(error, "read"))?,
            draft: None,
            errors: vec![],
            receipt: Some(receipt),
            validation_origin: None,
        });
    }
    if input.edits.is_empty() || input.edits.len() > 64 {
        return Err(transaction::before(
            Error::Invalid("patch需包含1–64处精确替换".into()),
            "validation",
        ));
    }
    let draft = repository::get_draft(db, &input.resource_id, input.conversation_id.as_deref())
        .map_err(|error| transaction::before(error, "read"))?
        .ok_or_else(|| transaction::before(Error::NotFound, "read"))?;
    if draft.id != input.draft_id || draft.revision != input.expected_draft_revision {
        return Err(transaction::before(Error::Conflict, "validation"));
    }
    let mut raw = draft.content;
    for edit in input.edits {
        if edit.old_text.is_empty() || raw.matches(&edit.old_text).count() != 1 {
            return Err(transaction::before(
                Error::Invalid("patch旧文须在当前草稿中唯一匹配；草稿保留".into()),
                "validation",
            ));
        }
        raw = raw.replacen(&edit.old_text, &edit.new_text, 1);
    }
    execute_with(
        sources,
        db,
        WriteDraft {
            resource_id: input.resource_id,
            expected_revision: input.expected_revision,
            content: raw,
            request_id: input.request_id,
            conversation_id: input.conversation_id,
            expected_draft_revision: Some(input.expected_draft_revision),
            source_snapshot_id: None,
            submission_id: input.submission_id,
            editor_kind: "agent".into(),
        },
        Some(identity),
    )
}

pub(super) fn receipt(
    db: &Connection,
    request: &str,
    request_hash: &str,
) -> Result<Option<Receipt>, Error> {
    let row=db.query_row("select resource_id,revision,content_hash,request_hash from structured_resource_receipts where request_id=?1",[request],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?;
    row.map(|r| {
        if r.3 != request_hash {
            Err(Error::Invalid("请求身份已用于不同内容".into()))
        } else {
            Ok(Receipt {
                request_id: request.into(),
                resource_id: r.0,
                revision: r.1,
                content_hash: r.2,
            })
        }
    })
    .transpose()
}
pub(super) fn save_receipt(
    db: &Connection,
    receipt: &Receipt,
    operation: &str,
    request_hash: &str,
    conversation: Option<&str>,
    submission: Option<&str>,
) -> Result<(), Error> {
    db.execute("insert into structured_resource_receipts(request_id,resource_id,revision,content_hash,operation,request_hash,conversation_id,submission_id) values(?1,?2,?3,?4,?5,?6,?7,?8)",params![receipt.request_id,receipt.resource_id,receipt.revision,receipt.content_hash,operation,request_hash,conversation,submission])?;
    Ok(())
}
pub(crate) fn applied_content_for_submission(
    db: &Connection,
    conversation: &str,
    submission: &str,
    resource: &str,
) -> Result<Option<Value>, Error> {
    let raw:Option<String>=db.query_row("select v.content_json from structured_resource_receipts r join structured_resource_revisions v on v.resource_id=r.resource_id and v.revision=r.revision where r.conversation_id=?1 and r.submission_id=?2 and r.resource_id=?3 and r.operation='write' order by r.revision desc limit 1",params![conversation,submission,resource],|r|r.get(0)).optional()?.flatten();
    raw.as_deref().map(decode).transpose()
}
fn operation(
    db: &Connection,
    request: &str,
    fingerprint: &str,
) -> Result<Option<WriteResult>, Error> {
    let row=db.query_row("select request_hash,result_json from structured_resource_operations where request_id=?1",[request],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()?;
    row.map(|(stored, result)| {
        if stored != fingerprint {
            Err(Error::Invalid("请求身份已用于不同操作".into()))
        } else {
            decode(&result)
        }
    })
    .transpose()
}
fn save_operation(
    db: &Connection,
    input: &WriteDraft,
    fingerprint: &str,
    result: &WriteResult,
) -> Result<(), Error> {
    db.execute("insert into structured_resource_operations(request_id,resource_id,request_hash,result_json,conversation_id,submission_id) values(?1,?2,?3,?4,?5,?6)",params![input.request_id,input.resource_id,fingerprint,encode(result)?,input.conversation_id,input.submission_id])?;
    Ok(())
}
pub(crate) fn query_outcome(db: &Connection, request: &str) -> Result<Option<WriteResult>, Error> {
    db.query_row(
        "select result_json from structured_resource_operations where request_id=?1",
        [request],
        |r| r.get::<_, String>(0),
    )
    .optional()?
    .as_deref()
    .map(decode)
    .transpose()
}
