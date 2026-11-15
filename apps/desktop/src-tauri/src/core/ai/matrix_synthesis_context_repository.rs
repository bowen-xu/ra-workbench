use super::{error::AiError, model::AiMatrixSynthesisContext};
use crate::ai::research_input_contracts::DerivedViewInputRef;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
pub(crate) fn insert_matrix_synthesis_context(
    transaction: &Transaction<'_>,
    run_id: &str,
    context: &AiMatrixSynthesisContext,
) -> Result<(), AiError> {
    transaction.execute(
        "insert into ai_matrix_synthesis_run_contexts
         (run_id, project_id, matrix_id, matrix_revision, view_kind, title,
          scope_note, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            run_id,
            context.project_id,
            context.matrix_id,
            context.matrix_revision,
            context.view_kind,
            context.title,
            context.scope_note,
        ],
    )?;
    for (ordinal, input) in context.inputs.iter().enumerate() {
        transaction.execute(
            "insert into ai_matrix_synthesis_input_cells
             (run_id, ordinal, cell_id, cell_revision, selection_reason, created_at)
             values (?1, ?2, ?3, ?4, ?5,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
            params![
                run_id,
                i64::try_from(ordinal).map_err(|_| invalid("综合输入数量超限"))?,
                input.object_id,
                input.object_revision,
                input.selection_reason.trim(),
            ],
        )?;
    }
    Ok(())
}
pub(crate) fn load_matrix_synthesis_context(
    connection: &Connection,
    run_id: &str,
) -> Result<Option<AiMatrixSynthesisContext>, AiError> {
    let header = connection
        .query_row(
            "select project_id, matrix_id, matrix_revision, view_kind, title, scope_note
             from ai_matrix_synthesis_run_contexts where run_id = ?1",
            [run_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
        .optional()?;
    let Some(header) = header else {
        return Ok(None);
    };
    let mut statement = connection.prepare(
        "select cell_id, cell_revision, selection_reason
         from ai_matrix_synthesis_input_cells where run_id = ?1 order by ordinal",
    )?;
    let inputs = statement
        .query_map([run_id], |row| {
            Ok(DerivedViewInputRef {
                object_kind: "matrix_cell".to_owned(),
                object_id: row.get(0)?,
                object_revision: row.get(1)?,
                selection_reason: row.get(2)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut scope = connection.prepare(
        "select object_id from ai_scope_items
         where run_id = ?1 and item_kind = 'anchor' order by ordinal",
    )?;
    let evidence_anchor_ids = scope
        .query_map([run_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(AiMatrixSynthesisContext {
        project_id: header.0,
        matrix_id: header.1,
        matrix_revision: header.2,
        view_kind: header.3,
        title: header.4,
        scope_note: header.5,
        inputs,
        evidence_anchor_ids,
    }))
}
fn invalid(message: &str) -> AiError {
    AiError::InvalidInput(message.to_owned())
}
