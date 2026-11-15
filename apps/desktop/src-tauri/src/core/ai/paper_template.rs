use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::{error::AiError, model::AiOverviewProfileContext};

pub(crate) use crate::ai::paper_analysis_contracts::PaperTemplateRef;

pub(crate) use super::candidate_contracts::{
    PaperTemplateSnapshot, valid_template_snapshot as valid_snapshot,
};

/// New submissions use adopted resource revisions. Historical pre-resource
/// adapters retain their original resolution semantics and immutable snapshots.
pub(crate) fn resolve(
    policy: super::generation_policy::GenerationPolicySource,
    connection: &Connection,
    paper: &str,
    reference: &PaperTemplateRef,
) -> Result<AiOverviewProfileContext, AiError> {
    (policy.template_resolve)(connection, paper, reference)
}

pub(crate) fn insert_run(
    transaction: &Transaction<'_>,
    run_id: &str,
    context: &AiOverviewProfileContext,
) -> Result<(), AiError> {
    let template = context
        .paper_template
        .as_ref()
        .ok_or(AiError::TemporaryUnavailable)?;
    if matches!(template.reference, PaperTemplateRef::Resource { .. }) {
        return super::resource_template::insert_run(transaction, run_id, context);
    }
    transaction.execute("insert into ai_overview_template_runs (run_id, paper_id, portrait_candidate_id, portrait_revision, template_json, created_at)
        values (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![run_id, context.paper_id, template.reference.id(), template.reference.revision(), serde_json::to_string(template).map_err(|_| AiError::TemporaryUnavailable)?])?;
    Ok(())
}

pub(crate) fn load_run(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<AiOverviewProfileContext>, AiError> {
    if let Some(context) = super::resource_template::load_run(connection, run_id)? {
        return Ok(Some(context));
    }
    let stored: Option<(String, String)> = connection
        .query_row(
            "select paper_id, template_json from ai_overview_template_runs where run_id = ?1",
            [run_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    stored
        .map(|(paper_id, json)| {
            let template: PaperTemplateSnapshot =
                serde_json::from_str(&json).map_err(|_| AiError::TemporaryUnavailable)?;
            if !valid_snapshot(&template) {
                return Err(AiError::TemporaryUnavailable);
            }
            Ok(AiOverviewProfileContext {
                paper_id,
                selection_revision: template.reference.revision(),
                profile_refs: vec![],
                field_kinds: template
                    .fields
                    .iter()
                    .map(|field| field.id.clone())
                    .collect(),
                paper_template: Some(template),
            })
        })
        .transpose()
}
