use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use super::{MatrixError as SynthesisError, MatrixPolicy, MatrixSql};
use research_workbench_native_sdk::evidence_matrix::{
    EvidenceMatrix, EvidenceMatrixCell, EvidenceMatrixCellDraft, EvidenceMatrixDraft,
    MatrixValidationInputV1,
};

pub(crate) fn save_matrix_from(
    sql: &MatrixSql,
    policy: &mut dyn MatrixPolicy,
    explicit: bool,
    connection: &mut Connection,
    draft: EvidenceMatrixDraft,
) -> Result<EvidenceMatrix, SynthesisError> {
    validate_matrix(sql, policy, connection, &draft)?;
    let mut identity = draft.id.clone();
    let transaction =
        Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Deferred)?;
    let result = (|| {
        let id = match draft.id.as_deref() {
            None => insert_matrix(sql, &transaction, &draft, &mut identity)?,
            Some(id) => update_matrix(sql, &transaction, id, &draft)?,
        };
        Ok(id)
    })();
    let id = super::settle(connection, transaction, result, explicit).map_err(|cause| {
        match (explicit, identity) {
            (true, Some(id)) => SynthesisError::Identified {
                id,
                cause: Box::new(cause),
            },
            _ => cause,
        }
    })?;
    matrix_from(sql, connection, &id).map_err(|cause| {
        if explicit {
            SynthesisError::Readback {
                id,
                revision: None,
                cause: Box::new(cause),
            }
        } else {
            cause
        }
    })
}

pub(crate) fn list_matrices_from(
    sql: &MatrixSql,
    connection: &Connection,
    project_id: &str,
) -> Result<Vec<EvidenceMatrix>, SynthesisError> {
    let mut statement = connection.prepare(sql.get("ids"))?;
    let ids = statement
        .query_map([project_id], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    ids.iter()
        .map(|id| matrix_from(sql, connection, id))
        .collect()
}

pub(crate) fn save_matrix_cell_from(
    sql: &MatrixSql,
    policy: &mut dyn MatrixPolicy,
    explicit: bool,
    connection: &mut Connection,
    draft: EvidenceMatrixCellDraft,
) -> Result<EvidenceMatrixCell, SynthesisError> {
    validate_cell(sql, policy, connection, &draft)?;
    let mut identity = None;
    let transaction =
        Transaction::new_unchecked(connection, rusqlite::TransactionBehavior::Deferred)?;
    let result = (|| {
        let current = transaction
            .query_row(
                sql.get("cell-header"),
                params![draft.matrix_id, draft.paper_id, draft.dimension_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?;
        let (id, revision) = match current {
            None => insert_cell(sql, &transaction, &draft, &mut identity)?,
            Some((id, revision)) => {
                identity = Some(id.clone());
                update_cell(sql, &transaction, &id, revision, &draft)?
            }
        };
        Ok((id, revision))
    })();
    let (id, revision) =
        super::settle(connection, transaction, result, explicit).map_err(|cause| {
            match (explicit, identity) {
                (true, Some(id)) => SynthesisError::Identified {
                    id,
                    cause: Box::new(cause),
                },
                _ => cause,
            }
        })?;
    cell_from(sql, connection, &id, revision).map_err(|cause| {
        if explicit {
            SynthesisError::Readback {
                id,
                revision: Some(revision),
                cause: Box::new(cause),
            }
        } else {
            cause
        }
    })
}

pub(crate) fn list_matrix_cells_from(
    sql: &MatrixSql,
    connection: &Connection,
    matrix_id: &str,
) -> Result<Vec<EvidenceMatrixCell>, SynthesisError> {
    matrix_from(sql, connection, matrix_id)?;
    let mut statement = connection.prepare(sql.get("cell-keys"))?;
    let keys = statement
        .query_map([matrix_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    keys.iter()
        .map(|(id, revision)| cell_from(sql, connection, id, *revision))
        .collect()
}

fn insert_matrix(
    sql: &MatrixSql,
    transaction: &Transaction<'_>,
    draft: &EvidenceMatrixDraft,
    identity: &mut Option<String>,
) -> Result<String, SynthesisError> {
    if draft.expected_revision.is_some() {
        return Err(SynthesisError::InvalidInput(
            "新证据矩阵不能携带预期版本".to_owned(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    *identity = Some(id.clone());
    let paper_ids_json = json(&draft.paper_ids, "论文范围")?;
    let dimensions_json = json(&draft.dimensions, "比较维度")?;
    transaction.execute(
        sql.get("insert"),
        params![
            id,
            draft.project_id,
            draft.title.trim(),
            paper_ids_json,
            dimensions_json,
        ],
    )?;
    insert_matrix_revision(sql, transaction, &id, 1, draft)?;
    Ok(id)
}

fn update_matrix(
    sql: &MatrixSql,
    transaction: &Transaction<'_>,
    id: &str,
    draft: &EvidenceMatrixDraft,
) -> Result<String, SynthesisError> {
    let (project_id, current_revision) = transaction
        .query_row(sql.get("header"), [id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .optional()?
        .ok_or(SynthesisError::NotFound)?;
    if project_id != draft.project_id {
        return Err(SynthesisError::InvalidInput(
            "证据矩阵不能移动到另一项目".to_owned(),
        ));
    }
    if draft.expected_revision != Some(current_revision) {
        return Err(SynthesisError::RevisionConflict { current_revision });
    }
    let next_revision = current_revision + 1;
    let changed = transaction.execute(
        sql.get("update"),
        params![
            id,
            draft.title.trim(),
            json(&draft.paper_ids, "论文范围")?,
            json(&draft.dimensions, "比较维度")?,
            next_revision,
            current_revision,
        ],
    )?;
    if changed != 1 {
        return Err(SynthesisError::RevisionConflict { current_revision });
    }
    insert_matrix_revision(sql, transaction, id, next_revision, draft)?;
    Ok(id.to_owned())
}

fn insert_matrix_revision(
    sql: &MatrixSql,
    transaction: &Transaction<'_>,
    id: &str,
    revision: i64,
    draft: &EvidenceMatrixDraft,
) -> Result<(), SynthesisError> {
    transaction.execute(
        sql.get("revision"),
        params![
            Uuid::new_v4().to_string(),
            id,
            revision,
            json(draft, "矩阵版本")?,
            draft.change_reason.trim(),
        ],
    )?;
    Ok(())
}

fn insert_cell(
    sql: &MatrixSql,
    transaction: &Transaction<'_>,
    draft: &EvidenceMatrixCellDraft,
    identity: &mut Option<String>,
) -> Result<(String, i64), SynthesisError> {
    if draft.expected_revision.is_some() {
        return Err(SynthesisError::InvalidInput(
            "新矩阵单元不能携带预期版本".to_owned(),
        ));
    }
    let id = Uuid::new_v4().to_string();
    *identity = Some(id.clone());
    transaction.execute(
        sql.get("cell-insert"),
        params![
            id,
            draft.matrix_id,
            draft.paper_id,
            draft.dimension_id,
            draft.coverage_state,
            draft.coverage_note.trim(),
            json(&draft.statements, "矩阵单元陈述")?,
        ],
    )?;
    insert_cell_revision(sql, transaction, &id, 1, draft)?;
    Ok((id, 1))
}

fn update_cell(
    sql: &MatrixSql,
    transaction: &Transaction<'_>,
    id: &str,
    current_revision: i64,
    draft: &EvidenceMatrixCellDraft,
) -> Result<(String, i64), SynthesisError> {
    if draft.expected_revision != Some(current_revision) {
        return Err(SynthesisError::RevisionConflict { current_revision });
    }
    let next_revision = current_revision + 1;
    let changed = transaction.execute(
        sql.get("cell-update"),
        params![
            id,
            draft.coverage_state,
            draft.coverage_note.trim(),
            json(&draft.statements, "矩阵单元陈述")?,
            next_revision,
            current_revision,
        ],
    )?;
    if changed != 1 {
        return Err(SynthesisError::RevisionConflict { current_revision });
    }
    insert_cell_revision(sql, transaction, id, next_revision, draft)?;
    Ok((id.to_owned(), next_revision))
}

fn insert_cell_revision(
    sql: &MatrixSql,
    transaction: &Transaction<'_>,
    id: &str,
    revision: i64,
    draft: &EvidenceMatrixCellDraft,
) -> Result<(), SynthesisError> {
    transaction.execute(
        sql.get("cell-revision"),
        params![
            Uuid::new_v4().to_string(),
            id,
            revision,
            json(draft, "矩阵单元版本")?,
            draft.change_reason.trim(),
        ],
    )?;
    Ok(())
}

fn matrix_from(
    sql: &MatrixSql,
    connection: &Connection,
    id: &str,
) -> Result<EvidenceMatrix, SynthesisError> {
    connection
        .query_row(sql.get("matrix"), [id], |row| {
            let paper_ids_json = row.get::<_, String>(3)?;
            let dimensions_json = row.get::<_, String>(4)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                paper_ids_json,
                dimensions_json,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })
        .optional()?
        .ok_or(SynthesisError::NotFound)
        .and_then(|value| {
            Ok(EvidenceMatrix {
                id: value.0,
                project_id: value.1,
                title: value.2,
                paper_ids: decode_json(&value.3, "论文范围")?,
                dimensions: decode_json(&value.4, "比较维度")?,
                revision: value.5,
                created_at: value.6,
                updated_at: value.7,
            })
        })
}

fn cell_from(
    sql: &MatrixSql,
    connection: &Connection,
    id: &str,
    revision: i64,
) -> Result<EvidenceMatrixCell, SynthesisError> {
    connection
        .query_row(sql.get("cell"), params![id, revision], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
            ))
        })
        .optional()?
        .ok_or(SynthesisError::NotFound)
        .and_then(|value| {
            Ok(EvidenceMatrixCell {
                id: value.0,
                matrix_id: value.1,
                paper_id: value.2,
                dimension_id: value.3,
                coverage_state: value.4,
                coverage_note: value.5,
                statements: decode_json(&value.6, "矩阵单元陈述")?,
                provenance_kind: value.7,
                revision,
                created_at: value.8,
                updated_at: value.9,
            })
        })
}

fn validate_matrix(
    sql: &MatrixSql,
    policy: &mut dyn MatrixPolicy,
    connection: &Connection,
    draft: &EvidenceMatrixDraft,
) -> Result<(), SynthesisError> {
    policy.validate(MatrixValidationInputV1::Matrix {
        draft: draft.clone(),
    })?;
    for paper_id in &draft.paper_ids {
        let belongs = connection.query_row(
            sql.get("project-belongs"),
            params![draft.project_id, paper_id],
            |row| row.get::<_, bool>(0),
        )?;
        if !belongs {
            return Err(SynthesisError::InvalidInput(
                "证据矩阵只能选择当前项目中的文献".to_owned(),
            ));
        }
    }
    Ok(())
}

fn validate_cell(
    sql: &MatrixSql,
    policy: &mut dyn MatrixPolicy,
    connection: &Connection,
    draft: &EvidenceMatrixCellDraft,
) -> Result<(), SynthesisError> {
    let matrix = matrix_from(sql, connection, &draft.matrix_id)?;
    policy.validate(MatrixValidationInputV1::Cell {
        draft: draft.clone(),
        matrix,
    })?;
    let mut prior_statement_ids = Vec::new();
    for statement in &draft.statements {
        policy.validate(MatrixValidationInputV1::Statement {
            statement: statement.clone(),
            prior_statement_ids: prior_statement_ids.clone(),
        })?;
        for anchor_id in &statement.evidence_anchor_ids {
            let belongs = connection.query_row(
                sql.get("anchor-belongs"),
                params![anchor_id, draft.paper_id],
                |row| row.get::<_, bool>(0),
            )?;
            if !belongs {
                return Err(SynthesisError::InvalidInput(
                    "矩阵陈述证据必须来自当前单元的论文".to_owned(),
                ));
            }
        }
        prior_statement_ids.push(statement.id.clone());
    }
    Ok(())
}

fn json<T: serde::Serialize + ?Sized>(value: &T, label: &str) -> Result<String, SynthesisError> {
    serde_json::to_string(value)
        .map_err(|_| SynthesisError::InvalidInput(format!("{label}无法序列化")))
}

fn decode_json<T: serde::de::DeserializeOwned>(
    value: &str,
    label: &str,
) -> Result<T, SynthesisError> {
    serde_json::from_str(value)
        .map_err(|_| SynthesisError::InvalidInput(format!("{label}存储格式无效")))
}
