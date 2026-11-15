//! Human history replays a trusted D/B transition; it never accepts a client snapshot.
use super::review::{ReviewPolicy, ReviewRoute, boundary};
use super::{
    Error, HistoryDraft, Receipt, Resource, content, drafts, encode, get, hash, repository,
};
use crate::{
    ai::structured_resource_validation::StructuredPolicy,
    hbs_storage::boundary::{
        self as transaction_boundary, BoundaryError, HbsSources, StaticHbsSources,
    },
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

fn version(db: &Connection, id: &str, revision: i64) -> Result<Resource, Error> {
    if revision > 0 {
        repository::get_version(db, id, revision)
    } else {
        let mut empty = repository::get_header(db, id)?;
        empty.revision = 0;
        Ok(empty)
    }
}
fn protected(db: &Connection, id: &str, revision: i64) -> Result<Vec<String>, Error> {
    if revision == 0 {
        return Ok(vec![]);
    }
    let raw: String = db.query_row(
        "select protected_fields_json from structured_resource_revisions where resource_id=?1 and revision=?2",
        params![id, revision], |row| row.get(0),
    )?;
    super::decode(&raw)
}
fn same(db: &Connection, left: &Resource, right: &Resource) -> Result<bool, Error> {
    if left.content != right.content
        || left.review_baseline != right.review_baseline
        || left.reviewed_field_ids != right.reviewed_field_ids
        || encode(&left.template_ref)? != encode(&right.template_ref)?
        || protected(db, &left.id, left.revision)? != protected(db, &right.id, right.revision)?
    {
        return Ok(false);
    }
    for field in content::fields(left.content.as_ref()).into_keys() {
        if super::provenance::field_sources(db, &left.id, left.revision, &field)?
            != super::provenance::field_sources(db, &right.id, right.revision, &field)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}
pub(super) fn restore(
    policy: crate::ai::generation_policy::GenerationPolicySource,
    reading_source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &mut Connection,
    input: HistoryDraft,
) -> Result<Resource, Error> {
    restore_with(
        &mut StaticHbsSources::new(Some(policy), Some(reading_source)),
        db,
        input,
        ReviewRoute::Static,
    )
    .map_err(BoundaryError::into_static)
}

pub(crate) fn restore_with<S: HbsSources>(
    sources: &mut S,
    db: &mut Connection,
    input: HistoryDraft,
    route: ReviewRoute,
) -> Result<Resource, BoundaryError<<S::Policy as StructuredPolicy>::Pause, Error>>
where
    S::Policy: ReviewPolicy,
{
    if !["undo", "redo"].contains(&input.direction.as_str()) || input.operation_revision < 1 {
        return Err(boundary::before_transaction(
            Error::Invalid("无效的阅读撤销重做操作".into()),
            "validation",
        ));
    }
    let fingerprint =
        hash(&encode(&input).map_err(|error| boundary::before_transaction(error, "receipt"))?);
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| boundary::before_transaction(error, "begin"))?;
    let mut phase = "receipt";
    let result = (|| {
        if let Some(receipt) = drafts::receipt(&tx, &input.request_id, &fingerprint)? {
            let resource = get(&tx, &receipt.resource_id)?;
            route.history(&resource)?;
            return Ok((resource, false));
        }
        phase = "read";
        let current = get(&tx, &input.resource_id)?;
        if let Some(paper) = current.paper_id.as_deref() {
            super::legacy_guard::require_legacy_imported(&tx, paper)?;
        }
        if current.revision != input.expected_revision {
            return Err(Error::Conflict.into());
        }
        route.history(&current)?;
        let trusted: bool = tx.query_row(
        "select exists(select 1 from structured_resource_receipts r join structured_resource_revisions v on v.resource_id=r.resource_id and v.revision=r.revision where r.resource_id=?1 and r.revision=?2 and r.operation in ('write','review') and v.editor_kind in ('human','review'))",
        params![current.id, input.operation_revision], |row| row.get(0),
    )?;
        if !trusted {
            return Err(Error::Invalid("找不到对应的人工修改或审阅回执".into()).into());
        }
        let before = version(&tx, &current.id, input.operation_revision - 1)?;
        let after = version(&tx, &current.id, input.operation_revision)?;
        let (expected, target) = if input.direction == "undo" {
            (after, before)
        } else {
            (before, after)
        };
        if !same(&tx, &current, &expected)? {
            return Err(Error::Conflict.into());
        }
        if let Some(value) = target.content.as_ref() {
            phase = "validation";
            let mut validation = sources
                .require()
                .map_err(|error| error.map_error(Error::Invalid))?;
            super::validation::validate_review_with(&mut validation, &tx, &target, &encode(value)?)
                .map_err(|error| {
                    error.map_error(|_| {
                        Error::Invalid("历史正文无法通过校验，当前内容已保留".into())
                    })
                })?;
        }
        phase = "read";
        let source: Option<String> = tx.query_row(
        "select source_snapshot_id from structured_resource_revisions where resource_id=?1 and revision=?2",
        params![target.id, target.revision], |row| row.get(0),
    ).optional()?.flatten();
        let mut restored = target.clone();
        restored.revision = current.revision + 1;
        let protection = protected(&tx, &target.id, target.revision)?;
        phase = "write";
        repository::persist_revision(&tx, &restored, "review", source.as_deref(), &protection)?;
        // Restore exact field ownership, rather than inferring it from equal text in another run.
        tx.execute(
            "delete from structured_resource_field_sources where resource_id=?1 and revision=?2",
            params![current.id, restored.revision],
        )?;
        tx.execute("insert into structured_resource_field_sources(resource_id,revision,field_id,source_snapshot_id) select resource_id,?3,field_id,source_snapshot_id from structured_resource_field_sources where resource_id=?1 and revision=?2",params![current.id,target.revision,restored.revision])?;
        tx.execute("update structured_resource_revisions set source_snapshot_id=?3 where resource_id=?1 and revision=?2",params![current.id,restored.revision,source])?;
        phase = "receipt";
        drafts::save_receipt(
            &tx,
            &Receipt {
                request_id: input.request_id,
                resource_id: restored.id.clone(),
                revision: restored.revision,
                content_hash: hash(&encode(&restored.content)?),
            },
            "history",
            &fingerprint,
            None,
            None,
        )?;
        Ok((restored, true))
    })();
    let settled = boundary::settle(tx, result, phase, route.explicit());
    transaction_boundary::confirm_with(db, settled, route.explicit(), |message| {
        Error::Invalid(message.into())
    })
}
