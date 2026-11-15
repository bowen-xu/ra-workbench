use super::{
    Error, Receipt, Resource, ReviewDraft, content, decode, drafts, encode, get, hash, repository,
};
use crate::{
    ai::structured_resource_validation::{Flow, StructuredPolicy},
    hbs_storage::boundary::{
        self as transaction_boundary, BoundaryError, HbsSources, StaticHbsSources,
    },
};
use research_workbench_native_sdk::review_selection::UnreviewedSelectionInputV1;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};
#[path = "review/boundary.rs"]
pub(super) mod boundary;
pub(crate) use boundary::{ReviewPolicy, ReviewRoute};
#[cfg(test)]
#[path = "review/runtime_tests.rs"]
mod runtime_tests;

pub(crate) fn review(
    policy: crate::ai::generation_policy::GenerationPolicySource,
    reading_source: crate::ai::paper_reading_source::PaperReadingSource,
    db: &mut Connection,
    input: ReviewDraft,
) -> Result<Resource, Error> {
    review_with(
        &mut StaticHbsSources::new(Some(policy), Some(reading_source)),
        db,
        input,
        ReviewRoute::Static,
    )
    .map_err(BoundaryError::into_static)
}

pub(crate) fn review_with<S: HbsSources>(
    sources: &mut S,
    db: &mut Connection,
    input: ReviewDraft,
    route: ReviewRoute,
) -> Result<Resource, BoundaryError<<S::Policy as StructuredPolicy>::Pause, Error>>
where
    S::Policy: ReviewPolicy,
{
    if ![
        "keep",
        "undo",
        "undoGeneration",
        "verify",
        "resetVerification",
        "clear",
    ]
    .contains(&input.action.as_str())
        || !matches!(input.scope.as_deref(), None | Some("all" | "unreviewed"))
    {
        return Err(boundary::before_transaction(
            Error::Invalid("未知审阅操作".into()),
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
            route.review(&resource, &input)?;
            return Ok((resource, false));
        }
        phase = "read";
        let mut resource = get(&tx, &input.resource_id)?;
        if resource.revision != input.expected_revision {
            return Err(Error::Conflict.into());
        }
        route.review(&resource, &input)?;
        if input.action == "clear"
            && (resource.kind != "structuredReading"
                || input.field_ids.is_some()
                || input.scope.is_some())
        {
            return Err(Error::Invalid("清空操作只适用于整份结构化阅读记录".into()).into());
        }
        let mut before = resource.clone();
        let protected: String = tx.query_row(
            "select protected_fields_json from structured_resources where id=?1",
            [&resource.id],
            |r| r.get(0),
        )?;
        let mut protected: Vec<String> = decode(&protected)?;
        let pending = resource.pending_field_ids.clone();
        let verifying = ["verify", "resetVerification"].contains(&input.action.as_str());
        let available = if verifying {
            content::fields(resource.content.as_ref())
                .into_keys()
                .collect::<Vec<_>>()
        } else {
            pending.clone()
        };
        let mut selected = input.field_ids.unwrap_or_else(|| available.clone());
        if selected.iter().any(|id| !available.contains(id)) {
            return Err(Error::Invalid("审阅字段已变化，请重新读取".into()).into());
        }
        if input.scope.as_deref() == Some("unreviewed") {
            phase = "validation";
            let policy = sources
                .policy_only()
                .map_err(|error| error.map_error(Error::Invalid))?;
            selected = policy
                .select_unreviewed(UnreviewedSelectionInputV1 {
                    resource_id: resource.id.clone(),
                    resource_revision: resource.revision,
                    pending_field_ids: selected,
                    protected_field_ids: protected.clone(),
                    reviewed_field_ids: resource.reviewed_field_ids.clone(),
                })
                .map_err(Flow::Need)?;
        }
        if input.action == "clear" {
            // A durable empty D/B makes even identical regeneration a fresh addition.
            // Keep the stable identity, pinned template, prior revisions and repair draft.
            resource.content = None;
            resource.review_baseline = None;
            resource.reviewed_field_ids.clear();
            protected.clear();
        } else if input.action == "undoGeneration" {
            restore_clean_generation(&tx, &mut resource)?;
            selected = content::fields(resource.content.as_ref())
                .into_keys()
                .chain(available)
                .collect();
        } else if input.action == "verify" {
            for id in &selected {
                if !resource.reviewed_field_ids.contains(id) {
                    resource.reviewed_field_ids.push(id.clone());
                }
            }
        } else if input.action == "resetVerification" {
            resource
                .reviewed_field_ids
                .retain(|id| !selected.contains(id));
        } else if input.action == "keep" {
            content::replace_fields(
                &mut resource.review_baseline,
                resource.content.as_ref(),
                &selected,
                true,
            );
        } else if selected.iter().collect::<std::collections::BTreeSet<_>>()
            == pending.iter().collect::<std::collections::BTreeSet<_>>()
        {
            resource.content = resource.review_baseline.clone();
        } else {
            content::replace_fields(
                &mut resource.content,
                resource.review_baseline.as_ref(),
                &selected,
                false,
            );
        }
        // Keep is an editing decision, never a fabricated source-verification record.
        if !verifying {
            protected.retain(|id| !selected.contains(id));
        }
        if ["undo", "undoGeneration"].contains(&input.action.as_str()) {
            if let Some(value) = resource.content.as_ref() {
                before.template_ref = resource.template_ref.clone();
                phase = "validation";
                let mut validation = sources
                    .require()
                    .map_err(|error| error.map_error(Error::Invalid))?;
                super::validation::validate_review_with(&mut validation, &tx,&before,&encode(value)?).map_err(|error|error.map_error(|_|Error::Invalid("无法单独撤销所选部分，否则正式内容将不符合约定格式。请使用撤销快捷键回退之前的审阅操作，再使用 Undo all。".into())))?;
            }
            phase = "read";
            restore_verification(&tx, &mut resource, &selected)?;
        }
        phase = "write";
        resource.revision += 1;
        resource.pending_field_ids =
            content::pending(resource.content.as_ref(), resource.review_baseline.as_ref());
        if resource.kind == "readingTemplate"
            && let Some(name) = resource
                .content
                .as_ref()
                .and_then(|value| value.get("name"))
                .and_then(serde_json::Value::as_str)
        {
            tx.execute(
                "update structured_resources set title=?2 where id=?1",
                rusqlite::params![resource.id, name],
            )?;
            resource.title = name.to_owned();
        }
        phase = "read";
        let restored_source: Option<String> = if ["undo", "undoGeneration"]
            .contains(&input.action.as_str())
        {
            tx.query_row("select source_snapshot_id from structured_resource_revisions where resource_id=?1 and content_json is ?2 order by revision desc limit 1",rusqlite::params![resource.id,resource.content.as_ref().map(encode).transpose()?],|r|r.get::<_,Option<String>>(0)).optional()?.flatten()
        } else {
            None
        };
        phase = "write";
        repository::persist_revision(
            &tx,
            &resource,
            "review",
            restored_source.as_deref(),
            &protected,
        )?;
        phase = "receipt";
        drafts::save_receipt(
            &tx,
            &Receipt {
                request_id: input.request_id,
                resource_id: resource.id.clone(),
                revision: resource.revision,
                content_hash: hash(&encode(&resource.content)?),
            },
            "review",
            &fingerprint,
            None,
            None,
        )?;
        Ok((resource, true))
    })();
    let settled = boundary::settle(tx, result, phase, route.explicit());
    transaction_boundary::confirm_with(db, settled, route.explicit(), |message| {
        Error::Invalid(message.into())
    })
}

fn restore_clean_generation(db: &Connection, resource: &mut Resource) -> Result<(), Error> {
    let mut stmt=db.prepare("select revision from structured_resource_revisions where resource_id=?1 order by revision desc")?;
    let revisions = stmt
        .query_map([&resource.id], |r| r.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for revision in revisions {
        let previous = repository::get_version(db, &resource.id, revision)?;
        if content::pending(previous.content.as_ref(), previous.review_baseline.as_ref()).is_empty()
        {
            resource.content = previous.content;
            resource.review_baseline = previous.review_baseline;
            resource.template_ref = previous.template_ref;
            resource.reviewed_field_ids = previous.reviewed_field_ids;
            return Ok(());
        }
    }
    resource.content = None;
    resource.review_baseline = None;
    resource.reviewed_field_ids.clear();
    Ok(())
}

fn restore_verification(
    db: &Connection,
    resource: &mut Resource,
    ids: &[String],
) -> Result<(), Error> {
    let current = content::fields(resource.content.as_ref());
    let mut stmt=db.prepare("select content_json,reviewed_fields_json from structured_resource_revisions where resource_id=?1 order by revision desc")?;
    let versions = stmt
        .query_map([&resource.id], |r| {
            Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for id in ids {
        resource.reviewed_field_ids.retain(|field| field != id);
        if !current.contains_key(id) {
            continue;
        }
        for (raw, reviewed) in &versions {
            let value: Option<serde_json::Value> = raw.as_deref().map(decode).transpose()?;
            let previous = content::fields(value.as_ref());
            let alias = id
                .strip_prefix("$retained:")
                .map(str::to_owned)
                .unwrap_or_else(|| format!("$retained:{id}"));
            let key = if previous.get(id) == current.get(id) {
                Some(id)
            } else if previous.get(&alias) == current.get(id) {
                Some(&alias)
            } else {
                None
            };
            if let Some(key) = key {
                if decode::<Vec<String>>(reviewed)?.contains(key) {
                    resource.reviewed_field_ids.push(id.clone());
                }
                break;
            }
        }
    }
    Ok(())
}
