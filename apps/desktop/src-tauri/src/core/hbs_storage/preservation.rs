use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Observed {
    version: i64,
    saved: Value,
    baseline: Value,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Checkpoint {
    format: u8,
    kind: String,
    observed: Observed,
    content: Value,
    ancestor: Value,
    #[serde(
        default,
        deserialize_with = "optional_document",
        skip_serializing_if = "Option::is_none"
    )]
    review_ancestor: Option<Value>,
    baseline_ancestor: Value,
    ordinary: Vec<Value>,
}

fn optional_document<'de, D: serde::Deserializer<'de>>(
    value: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(value).map(Some)
}

use super::boundary::{self, BoundaryError, HbsOutcome, HbsSources, StaticHbsSources};
use crate::ai::structured_resource_validation::{Failure, Flow, StructuredPolicy};

fn validate_with<S: HbsSources>(
    sources: &mut S,
    db: &Connection,
    kind: &str,
    id: &str,
    value: Value,
) -> Result<Value, Flow<String, <S::Policy as StructuredPolicy>::Pause>> {
    let checkpoint: Checkpoint = serde_json::from_value(value)
        .map_err(|error| Flow::original(format!("MODEL_RECOVERY_INVALID：{error}")))?;
    if checkpoint.format != 1 || checkpoint.kind != kind || checkpoint.observed.version < 0 {
        return Err(Flow::original(
            "MODEL_RECOVERY_INVALID：身份或修订无效".into(),
        ));
    }
    for value in [
        &checkpoint.content,
        &checkpoint.ancestor,
        &checkpoint.baseline_ancestor,
        &checkpoint.observed.saved,
        &checkpoint.observed.baseline,
    ]
    .into_iter()
    .chain(checkpoint.review_ancestor.iter())
    {
        match kind {
            "text" if value.is_string() => {}
            "note" => {
                let document: crate::notes::NoteDocument = serde_json::from_value(value.clone())
                    .map_err(|error| Flow::original(format!("MODEL_RECOVERY_INVALID：{error}")))?;
                if document.page_id != id {
                    return Err(Flow::original(
                        "MODEL_RECOVERY_INVALID：不能跨页面恢复".into(),
                    ));
                }
                crate::notes::document_validation::validate_document(&document)
                    .map_err(|error| Flow::original(format!("MODEL_RECOVERY_INVALID：{error}")))?;
            }
            "structured" => {
                crate::structured_resources::hbs::prepare_validation_sources(sources, db, id)?;
                crate::structured_resources::hbs::validate_preserved_with(sources, db, id, value)?;
            }
            _ => {
                return Err(Flow::original(
                    "MODEL_RECOVERY_INVALID：正文类型无效".into(),
                ));
            }
        }
    }
    for change in &checkpoint.ordinary {
        if !change.is_object()
            || !change["id"].is_string()
            || !change["path"].is_array()
            || !change["beforePath"].is_array()
        {
            return Err(Flow::original(
                "MODEL_RECOVERY_INVALID：普通输入投影范围无效".into(),
            ));
        }
    }
    serde_json::to_value(checkpoint).map_err(|error| Flow::original(error.to_string()))
}

pub(crate) fn read(
    policy: impl Into<Option<crate::ai::generation_policy::GenerationPolicySource>>,
    reading_source: impl Into<Option<crate::ai::paper_reading_source::PaperReadingSource>>,
    db: &Connection,
    kind: &str,
    id: &str,
) -> Result<Value, String> {
    read_with(
        &mut StaticHbsSources::new(policy.into(), reading_source.into()),
        db,
        kind,
        id,
    )
    .map_err(BoundaryError::into_static)
}
pub(crate) fn read_with<S: HbsSources>(
    sources: &mut S,
    db: &Connection,
    kind: &str,
    id: &str,
) -> Result<Value, BoundaryError<<S::Policy as StructuredPolicy>::Pause>> {
    let mut phase = "read";
    let result = read_flow(sources, db, kind, id, &mut phase);
    boundary::read_result(db, result, phase)
}
fn read_flow<S: HbsSources>(
    sources: &mut S,
    db: &Connection,
    kind: &str,
    id: &str,
    phase: &mut &'static str,
) -> Result<Value, Flow<String, <S::Policy as StructuredPolicy>::Pause>> {
    super::read_failure(db, kind, id).map_err(Flow::Fail)?;
    let raw: Option<String> = db
        .query_row(
            "select checkpoint_json from hbs_preservations where kind=?1 and resource_id=?2",
            params![kind, id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| Flow::Fail(Failure::sql(error.to_string(), "read", &error)))?;
    let value = raw
        .map(|raw| {
            *phase = "validation";
            serde_json::from_str(&raw)
                .map_err(|error| Flow::original(format!("MODEL_RECOVERY_INVALID：{error}")))
        })
        .transpose()?;
    if let Some(value) = value {
        return validate_with(sources, db, kind, id, value);
    }
    if kind == "note"
        && let Some(value) = crate::agent::text_tools::note_review::legacy_hbs_checkpoint(db, id)
            .map_err(Flow::original)?
    {
        *phase = "validation";
        return validate_with(sources, db, kind, id, value);
    }
    Ok(Value::Null)
}
/// S-prime commits before attempting S, with no requirement to resolve conflicts.
pub(crate) fn save(
    policy: impl Into<Option<crate::ai::generation_policy::GenerationPolicySource>>,
    reading_source: impl Into<Option<crate::ai::paper_reading_source::PaperReadingSource>>,
    db: &mut Connection,
    kind: &str,
    id: &str,
    value: Value,
) -> Result<Value, String> {
    save_with(
        &mut StaticHbsSources::new(policy.into(), reading_source.into()),
        db,
        kind,
        id,
        value,
    )
    .map_err(BoundaryError::into_static)
}
pub(crate) fn save_with<S: HbsSources>(
    sources: &mut S,
    db: &mut Connection,
    kind: &str,
    id: &str,
    value: Value,
) -> Result<Value, BoundaryError<<S::Policy as StructuredPolicy>::Pause>> {
    let explicit = sources.explicit_settlement();
    let tx = db.transaction().map_err(|error| {
        boundary::failure(
            Failure::sql(error.to_string(), "begin", &error),
            "begin",
            HbsOutcome::NotStarted,
        )
    })?;
    let mut phase = "read";
    let result = (|| {
        super::read_failure(&tx, kind, id).map_err(Flow::Fail)?;
        phase = "validation";
        let checkpoint = validate_with(sources, &tx, kind, id, value)?;
        let raw = checkpoint.to_string();
        phase = "write";
        tx.execute("insert into hbs_preservations(kind,resource_id,checkpoint_json) values(?1,?2,?3) on conflict(kind,resource_id) do update set checkpoint_json=excluded.checkpoint_json,revision=hbs_preservations.revision+1,updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now')",params![kind,id,raw]).map_err(|error|Flow::Fail(Failure::sql(error.to_string(),"write",&error)))?;
        tx.execute(
            "delete from hbs_preservation_assets where kind=?1 and resource_id=?2",
            params![kind, id],
        )
        .map_err(|error| Flow::Fail(Failure::sql(error.to_string(), "write", &error)))?;
        if kind == "note" {
            tx.execute("insert into hbs_preservation_assets(kind,resource_id,content_sha256) select ?1,?2,cast(value as text) from json_tree(?3) where key='assetId' group by value",params![kind,id,raw]).map_err(|error|Flow::Fail(Failure::sql(error.to_string(),"write",&error)))?;
        }
        let revision: i64 = tx
            .query_row(
                "select revision from hbs_preservations where kind=?1 and resource_id=?2",
                params![kind, id],
                |row| row.get(0),
            )
            .map_err(|error| Flow::Fail(Failure::sql(error.to_string(), "read", &error)))?;
        Ok(json!({"revision":revision}))
    })();
    let settled = boundary::settle(tx, result, phase, explicit, true);
    boundary::confirm(db, settled, explicit)
}
#[cfg(test)]
pub(crate) fn clear(db: &mut Connection, kind: &str, id: &str) -> Result<Value, String> {
    super::read(db, kind, id)?;
    db.execute(
        "delete from hbs_preservations where kind=?1 and resource_id=?2",
        params![kind, id],
    )
    .map_err(|error| error.to_string())?;
    Ok(json!({"cleared":true}))
}
