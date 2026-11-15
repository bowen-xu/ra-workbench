//! Shared candidate confirmation ledger. Domain rules and reading-unit work
//! stay with literature; this module only reads/appends existing Core facts.
use super::{
    error::AiError,
    model::{AiOverviewConfirmationBatch, AiOverviewFieldConfirmation},
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

pub(super) fn list_overview_field_confirmations(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Vec<AiOverviewFieldConfirmation>, AiError> {
    let rows = connection
        .prepare(
            "select f.id, f.batch_id, f.candidate_id, f.candidate_revision,
                    f.field_kind, f.field_payload_hash, f.decision_kind,
                    f.target_reading_unit_id, f.target_applied_revision,
                    f.created_at, coalesce(u.created_at, r.created_at),
                    case when u.id is null then r.created_at else null end
             from ai_overview_field_confirmations f
             left join ai_overview_confirmation_undos u on u.batch_id = f.batch_id
             left join ai_overview_field_review_resets r on r.field_confirmation_id = f.id
             where f.candidate_id = ?1 order by f.created_at, f.id",
        )?
        .query_map([candidate_id], confirmation_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub(super) fn load_confirmation_batch(
    connection: &Connection,
    batch_id: &str,
) -> Result<AiOverviewConfirmationBatch, AiError> {
    let mut batch = connection
        .query_row(
            "select b.id, b.candidate_id, b.candidate_revision, b.scope_kind,
                    b.paper_id, b.project_id, b.idempotency_key, b.created_at,
                    u.created_at
             from ai_overview_confirmation_batches b
             left join ai_overview_confirmation_undos u on u.batch_id = b.id
             where b.id = ?1",
            [batch_id],
            |row| {
                let undone_at = row.get::<_, Option<String>>(8)?;
                Ok(AiOverviewConfirmationBatch {
                    id: row.get(0)?,
                    candidate_id: row.get(1)?,
                    candidate_revision: row.get(2)?,
                    scope_kind: row.get(3)?,
                    paper_id: row.get(4)?,
                    project_id: row.get(5)?,
                    idempotency_key: row.get(6)?,
                    created_at: row.get(7)?,
                    active: undone_at.is_none(),
                    undone_at,
                    field_confirmations: Vec::new(),
                })
            },
        )
        .optional()?
        .ok_or_else(|| AiError::InvalidInput("找不到确认批次".to_owned()))?;
    batch.field_confirmations = connection
        .prepare(
            "select f.id, f.batch_id, f.candidate_id, f.candidate_revision,
                    f.field_kind, f.field_payload_hash, f.decision_kind,
                    f.target_reading_unit_id, f.target_applied_revision,
                    f.created_at, coalesce(u.created_at, r.created_at),
                    case when u.id is null then r.created_at else null end
             from ai_overview_field_confirmations f
             left join ai_overview_confirmation_undos u on u.batch_id = f.batch_id
             left join ai_overview_field_review_resets r on r.field_confirmation_id = f.id
             where f.batch_id = ?1 order by f.created_at, f.id",
        )?
        .query_map([batch_id], confirmation_from_row)?
        .collect::<Result<Vec<_>, _>>()?;
    batch.active = batch.field_confirmations.iter().any(|field| field.active);
    Ok(batch)
}

fn confirmation_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AiOverviewFieldConfirmation> {
    let undone_at = row.get::<_, Option<String>>(10)?;
    Ok(AiOverviewFieldConfirmation {
        id: row.get(0)?,
        batch_id: row.get(1)?,
        candidate_id: row.get(2)?,
        candidate_revision: row.get(3)?,
        field_kind: row.get(4)?,
        field_payload_hash: row.get(5)?,
        decision_kind: row.get(6)?,
        target_reading_unit_id: row.get(7)?,
        target_applied_revision: row.get(8)?,
        created_at: row.get(9)?,
        active: undone_at.is_none(),
        undone_at,
        review_reset_at: row.get(11)?,
    })
}

pub(super) fn batch_for_key(db: &Connection, key: &str) -> Result<Option<String>, AiError> {
    Ok(db
        .query_row(
            "select id from ai_overview_confirmation_batches where idempotency_key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}
pub(super) fn stored_review_state(db: &Connection, id: &str) -> Result<String, AiError> {
    Ok(db.query_row(
        "select review_state from ai_candidates where id = ?1",
        [id],
        |row| row.get(0),
    )?)
}
pub(super) fn reset_key_exists(db: &Connection, key: &str) -> Result<bool, AiError> {
    Ok(db.query_row(
        "select exists(select 1 from ai_overview_field_review_resets where idempotency_key=?1)",
        [key],
        |row| row.get(0),
    )?)
}
pub(super) fn undo_for_key(db: &Connection, key: &str) -> Result<Option<String>, AiError> {
    Ok(db
        .query_row(
            "select batch_id from ai_overview_confirmation_undos where idempotency_key = ?1",
            [key],
            |row| row.get(0),
        )
        .optional()?)
}
pub(super) fn reset_for_key(db: &Connection, key: &str) -> Result<Option<String>, AiError> {
    Ok(db.query_row("select field_confirmation_id from ai_overview_field_review_resets where idempotency_key = ?1",[key],|row|row.get(0)).optional()?)
}
pub(super) fn undo_key_exists(db: &Connection, key: &str) -> Result<bool, AiError> {
    Ok(db.query_row(
        "select exists(select 1 from ai_overview_confirmation_undos where idempotency_key = ?1)",
        [key],
        |row| row.get(0),
    )?)
}
pub(super) fn is_batch_active(db: &Connection, id: &str) -> Result<bool, AiError> {
    Ok(db.query_row(
        "select not exists(select 1 from ai_overview_confirmation_undos where batch_id = ?1)",
        [id],
        |row| row.get(0),
    )?)
}
pub(super) fn previous_snapshot(db: &Connection, id: &str) -> Result<Option<String>, AiError> {
    Ok(db.query_row(
        "select previous_snapshot_json from ai_overview_field_confirmations where id = ?1",
        [id],
        |row| row.get(0),
    )?)
}
pub(super) fn previous_state(db: &Connection, id: &str) -> Result<String, AiError> {
    Ok(db.query_row("select previous_candidate_review_state from ai_overview_confirmation_batches where id = ?1",[id],|row|row.get(0))?)
}
pub(super) enum ActiveCount {
    DistinctFields,
    Confirmations,
}

pub(super) fn active_count(db: &Connection, id: &str, mode: ActiveCount) -> Result<i64, AiError> {
    let sql = if matches!(mode, ActiveCount::DistinctFields) {
        "select count(distinct f.field_kind)
         from ai_overview_field_confirmations f
         join ai_overview_confirmation_batches b on b.id = f.batch_id
         left join ai_overview_confirmation_undos u on u.batch_id = b.id
         left join ai_overview_field_review_resets r on r.field_confirmation_id = f.id
         where f.candidate_id = ?1 and u.id is null and r.id is null"
    } else {
        "select count(*) from ai_overview_field_confirmations f
         join ai_overview_confirmation_batches b on b.id = f.batch_id
         left join ai_overview_confirmation_undos u on u.batch_id = b.id
         left join ai_overview_field_review_resets r on r.field_confirmation_id = f.id
         where f.candidate_id = ?1 and u.id is null and r.id is null"
    };
    Ok(db.query_row(sql, [id], |row| row.get(0))?)
}
pub(super) fn has_human(db: &Connection, id: &str) -> Result<bool, AiError> {
    Ok(db.query_row("select exists(
                 select 1 from ai_overview_field_confirmations f
                 join ai_overview_confirmation_batches b on b.id = f.batch_id
                 left join ai_overview_confirmation_undos u on u.batch_id = b.id
                 left join ai_overview_field_review_resets r on r.field_confirmation_id = f.id
                 where f.candidate_id = ?1 and f.decision_kind = 'human_edited' and u.id is null and r.id is null
             )",[id],|row|row.get(0))?)
}
pub(crate) struct BatchInsert<'a> {
    pub id: &'a str,
    pub candidate: &'a str,
    pub revision: i64,
    pub scope: &'a str,
    pub paper: &'a str,
    pub project: Option<&'a str>,
    pub previous_state: &'a str,
    pub key: &'a str,
}
pub(crate) struct FieldInsert<'a> {
    pub id: &'a str,
    pub batch: &'a str,
    pub candidate: &'a str,
    pub revision: i64,
    pub field: &'a str,
    pub hash: &'a str,
    pub decision: &'a str,
    pub target: &'a str,
    pub applied_revision: i64,
    pub previous_snapshot: Option<String>,
}
pub(super) fn append_batch(tx: &Transaction<'_>, r: BatchInsert<'_>) -> Result<(), AiError> {
    tx.execute(
        "insert into ai_overview_confirmation_batches
         (id, candidate_id, candidate_revision, scope_kind, paper_id, project_id,
          previous_candidate_review_state, idempotency_key, created_at)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8,
                 strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            r.id,
            r.candidate,
            r.revision,
            r.scope,
            r.paper,
            r.project,
            r.previous_state,
            r.key
        ],
    )?;
    Ok(())
}
pub(super) fn append_field(tx: &Transaction<'_>, r: FieldInsert<'_>) -> Result<(), AiError> {
    tx.execute(
        "insert into ai_overview_field_confirmations
             (id, batch_id, candidate_id, candidate_revision, field_kind,
              field_payload_hash, decision_kind, target_reading_unit_id,
              target_applied_revision, previous_snapshot_json, created_at)
             values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
                     strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![
            r.id,
            r.batch,
            r.candidate,
            r.revision,
            r.field,
            r.hash,
            r.decision,
            r.target,
            r.applied_revision,
            r.previous_snapshot
        ],
    )?;
    Ok(())
}
pub(super) fn append_undo(
    tx: &Transaction<'_>,
    id: &str,
    batch: &str,
    key: &str,
) -> Result<(), AiError> {
    tx.execute(
        "insert into ai_overview_confirmation_undos
         (id, batch_id, idempotency_key, created_at)
         values (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
        params![id, batch, key],
    )?;
    Ok(())
}
pub(super) fn append_reset(
    tx: &Transaction<'_>,
    id: &str,
    field: &str,
    key: &str,
) -> Result<(), AiError> {
    tx.execute("insert into ai_overview_field_review_resets (id, field_confirmation_id, idempotency_key, created_at) values (?1, ?2, ?3, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",params![id,field,key])?;
    Ok(())
}
pub(super) fn set_review_state(
    tx: &Transaction<'_>,
    candidate: &str,
    state: &str,
) -> Result<(), AiError> {
    tx.execute(
        "update ai_candidates set review_state = ?2,
             updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') where id = ?1",
        params![candidate, state],
    )?;
    Ok(())
}
