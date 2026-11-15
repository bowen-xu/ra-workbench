//! Original matrix storage, shared by static composition and finite Native rules.
pub(crate) mod commands;
pub(crate) mod context;
mod error;
mod repository;
mod resources;
use crate::{
    ai::structured_resource_validation::Flow,
    hbs_storage::boundary::{self, BoundaryError},
};
pub(crate) use error::{MatrixDiagnostic, MatrixError};
pub(crate) use repository::{
    list_matrices_from, list_matrix_cells_from, save_matrix_cell_from, save_matrix_from,
};
use research_workbench_native_sdk::evidence_matrix::{
    EvidenceMatrix, EvidenceMatrixCell, EvidenceMatrixCellDraft, EvidenceMatrixDraft,
    MatrixValidationInputV1,
};
pub(crate) use resources::MatrixSql;
use rusqlite::{Connection, Transaction};
use std::convert::Infallible;

pub(crate) trait MatrixPolicy {
    fn validate(&mut self, input: MatrixValidationInputV1) -> Result<(), MatrixError>;
}

#[derive(Clone)]
pub(crate) enum MatrixRequest {
    List { project_id: String },
    ListCells { matrix_id: String },
    Save { draft: Box<EvidenceMatrixDraft> },
    SaveCell { draft: Box<EvidenceMatrixCellDraft> },
}
#[derive(Debug)]
pub(crate) enum MatrixResult {
    Matrices(Vec<EvidenceMatrix>),
    Cells(Vec<EvidenceMatrixCell>),
    Matrix(EvidenceMatrix),
    Cell(EvidenceMatrixCell),
}
pub(crate) fn execute(
    sql: &MatrixSql,
    db: &mut Connection,
    policy: &mut dyn MatrixPolicy,
    request: MatrixRequest,
) -> Result<MatrixResult, MatrixError> {
    match request {
        MatrixRequest::List { project_id } => {
            list_matrices_from(sql, db, &project_id).map(MatrixResult::Matrices)
        }
        MatrixRequest::ListCells { matrix_id } => {
            list_matrix_cells_from(sql, db, &matrix_id).map(MatrixResult::Cells)
        }
        MatrixRequest::Save { draft } => {
            save_matrix_from(sql, policy, true, db, *draft).map(MatrixResult::Matrix)
        }
        MatrixRequest::SaveCell { draft } => {
            save_matrix_cell_from(sql, policy, true, db, *draft).map(MatrixResult::Cell)
        }
    }
}
fn settle<T>(
    db: &Connection,
    tx: Transaction<'_>,
    result: Result<T, MatrixError>,
    explicit: bool,
) -> Result<T, MatrixError> {
    let result: Result<T, Flow<MatrixError, Infallible>> = result.map_err(Flow::original);
    let settled = boundary::settle_with(
        tx,
        result,
        "write",
        explicit,
        true,
        MatrixError::Database,
        |cause| MatrixError::InvalidInput(cause.into()),
    );
    match boundary::confirm_with(db, settled, explicit, |cause| {
        MatrixError::InvalidInput(cause.into())
    }) {
        Ok(value) => Ok(value),
        Err(BoundaryError::Pause(_)) => unreachable!("matrix writes never suspend"),
        Err(BoundaryError::Failure(failure)) if explicit => {
            Err(MatrixError::Settled(Box::new(failure)))
        }
        Err(BoundaryError::Failure(failure)) => Err(failure.failure.error),
    }
}
