//! Durable authority shared by typed adapters and Native validation.
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
#[path = "hbs_storage/boundary.rs"]
pub(crate) mod boundary;
#[cfg(test)]
#[path = "hbs_storage/inspection.rs"]
pub(crate) mod inspection;
#[path = "hbs_storage/preservation.rs"]
pub(crate) mod preservation;
use crate::ai::structured_resource_validation::{Failure, Flow, StructuredPolicy};
use boundary::{BoundaryError, HbsOutcome, HbsSources, StaticHbsSources};

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Commit {
    pub id: String,
    pub expected_version: i64,
    pub saved: Value,
    pub baseline: Value,
}
pub(crate) fn read(db: &Connection, kind: &str, id: &str) -> Result<Value, String> {
    match kind {
        "note" => crate::notes::hbs::read(db,id),
        "structured" => crate::structured_resources::hbs::read(db,id),
        "text" => db.query_row("select revision,content,baseline from hbs_text_documents where id=?1",[id],|row|Ok(json!({"version":row.get::<_,i64>(0)?,"saved":row.get::<_,String>(1)?,"baseline":row.get::<_,String>(2)?}))).map_err(|e|e.to_string()),
        _ => Err("MODEL_INVALID_KIND：未知文档类型".into()),
    }
}
pub(crate) fn commit(
    policy: impl Into<Option<crate::ai::generation_policy::GenerationPolicySource>>,
    reading_source: impl Into<Option<crate::ai::paper_reading_source::PaperReadingSource>>,
    db: &mut Connection,
    kind: &str,
    id: &str,
    request: Commit,
) -> Result<Value, String> {
    commit_with(
        &mut StaticHbsSources::new(policy.into(), reading_source.into()),
        db,
        kind,
        id,
        &request,
    )
    .map_err(BoundaryError::into_static)
}
pub(crate) fn read_failure(
    db: &Connection,
    kind: &str,
    id: &str,
) -> Result<Value, Failure<String>> {
    if kind == "structured" {
        crate::structured_resources::hbs::read_failure(db, id)
    } else {
        read(db, kind, id).map_err(Failure::original)
    }
}
pub(crate) fn commit_with<S: HbsSources>(
    sources: &mut S,
    db: &mut Connection,
    kind: &str,
    id: &str,
    request: &Commit,
) -> Result<Value, BoundaryError<<S::Policy as StructuredPolicy>::Pause>> {
    if request.id.is_empty() || request.id.len() > 256 || request.expected_version < 0 {
        return Err(boundary::failure(
            Failure::original("MODEL_INVALID_REQUEST：请求身份或版本无效".into()),
            "read",
            HbsOutcome::NotStarted,
        ));
    }
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(request).map_err(|error| boundary::failure(
                Failure::original(error.to_string()),
                "read",
                HbsOutcome::NotStarted
            ))?
        )
    );
    let explicit = sources.explicit_settlement();
    let tx = db.transaction().map_err(|error| {
        boundary::failure(
            Failure::sql(error.to_string(), "begin", &error),
            "begin",
            HbsOutcome::NotStarted,
        )
    })?;
    let mut phase = "receipt";
    let result = (|| {
        let old:Option<(String,String)>=tx.query_row("select fingerprint,result_json from hbs_commit_receipts where kind=?1 and resource_id=?2 and request_id=?3",params![kind,id,request.id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(|error|Flow::Fail(Failure::sql(error.to_string(),"receipt",&error)))?;
        if let Some((prior, result)) = old {
            if prior != fingerprint {
                return Err(Flow::original(
                    "MODEL_REQUEST_ID_REUSED：请求身份不能更改内容".into(),
                ));
            }
            return serde_json::from_str(&result)
                .map(|value| (value, false))
                .map_err(|error| Flow::original(error.to_string()));
        }
        phase = "read";
        let current = read_failure(&tx, kind, id).map_err(Flow::Fail)?;
        if current["version"].as_i64() != Some(request.expected_version) {
            return Ok((
                json!({"type":"conflict","requestId":request.id,"snapshot":current,"code":"MODEL_VERSION_CONFLICT"}),
                false,
            ));
        }
        let snapshot = match kind {
            "note" => crate::notes::hbs::commit(
                &tx,
                id,
                request.expected_version,
                request.saved.clone(),
                request.baseline.clone(),
            )
            .map_err(Flow::original)?,
            "structured" => {
                phase = "sources";
                crate::structured_resources::hbs::prepare_validation_sources(sources, &tx, id)?;
                phase = "validation";
                crate::structured_resources::hbs::commit_with(
                    sources,
                    &tx,
                    id,
                    request.expected_version,
                    request.saved.clone(),
                    request.baseline.clone(),
                )?
            }
            "text" => {
                let saved = request
                    .saved
                    .as_str()
                    .ok_or_else(|| Flow::original("MODEL_INVALID_DOCUMENT：文本正文无效".into()))?;
                let baseline = request
                    .baseline
                    .as_str()
                    .ok_or_else(|| Flow::original("MODEL_INVALID_DOCUMENT：文本基线无效".into()))?;
                phase = "write";
                tx.execute("update hbs_text_documents set revision=revision+1,content=?2,baseline=?3 where id=?1 and revision=?4",params![id,saved,baseline,request.expected_version]).map_err(|error|Flow::Fail(Failure::sql(error.to_string(),"write",&error)))?;
                read_failure(&tx, kind, id).map_err(Flow::Fail)?
            }
            _ => return Err(Flow::original("MODEL_INVALID_KIND：未知文档类型".into())),
        };
        let result = json!({"type":"receipt","requestId":request.id,"snapshot":snapshot});
        phase = "receipt";
        tx.execute("insert into hbs_commit_receipts(kind,resource_id,request_id,fingerprint,result_json) values(?1,?2,?3,?4,?5)",params![kind,id,request.id,fingerprint,result.to_string()]).map_err(|error|Flow::Fail(Failure::sql(error.to_string(),"receipt",&error)))?;
        Ok((result, true))
    })();
    let (result, commit) = match result {
        Ok((value, commit)) => (Ok(value), commit),
        Err(error) => (Err(error), false),
    };
    let settled = boundary::settle(tx, result, phase, explicit, commit);
    boundary::confirm(db, settled, explicit)
}

#[cfg(test)]
#[path = "hbs_storage/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "hbs_storage/kernel_tests.rs"]
mod kernel_tests;

#[cfg(test)]
#[path = "hbs_storage/test_sources.rs"]
pub(crate) mod test_sources;
