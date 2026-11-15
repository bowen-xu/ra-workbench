use super::*;
use crate::{
    ai::structured_resource_validation::{Failure, FailureCause},
    data_interface::guarded_transaction,
    hbs_storage::{self, boundary::BoundaryFailure},
    runtime_plugins::{
        literature_hbs::{
            OperationError, OperationFailure, StructuredOperation, StructuredOutput, map_boundary,
            operation_error,
        },
        native::SafeDatabaseError,
    },
    structured_resources::{self, ReviewRoute},
};

type Attempt = (
    RuntimeHbsSources,
    Result<StructuredOutput, BoundaryError<HbsNeed, OperationFailure>>,
);
fn failure(
    message: String,
    phase: &'static str,
    outcome: HbsOutcome,
    cause: FailureCause,
) -> BoundaryFailure<OperationFailure> {
    BoundaryFailure {
        failure: Failure {
            error: OperationFailure::Hbs(message),
            cause,
        },
        phase,
        outcome,
        rollback_error: None,
    }
}
pub(super) async fn attempt_operation(
    store: AppStore,
    id: String,
    operation: Arc<StructuredOperation>,
    sources: RuntimeHbsSources,
) -> Result<Attempt, OperationError> {
    tokio::task::spawn_blocking(move || run(store, &id, &operation, sources))
        .await
        .map_err(|_| OperationError::unknown())?
}
fn run(
    store: AppStore,
    id: &str,
    operation: &StructuredOperation,
    mut sources: RuntimeHbsSources,
) -> Result<Attempt, OperationError> {
    let _content_read = if matches!(operation, StructuredOperation::Hbs(value) if matches!(value.as_ref(), HbsOperation::ReadPreservation))
    {
        Some(store.content_read().map_err(|error| {
            operation_error(failure(
                error.to_string(),
                "read",
                HbsOutcome::NotStarted,
                FailureCause::Original,
            ))
        })?)
    } else {
        None
    };
    let mut db = store.connection().map_err(|error| {
        operation_error(failure(
            error.to_string(),
            "begin",
            HbsOutcome::NotStarted,
            FailureCause::Original,
        ))
    })?;
    let result = guarded_transaction::run_mut(&mut db, |db| {
        let result = match operation {
            StructuredOperation::BindTemplate(input) => {
                structured_resources::adopt_template_with(&mut sources, db, input.clone())
                    .map(|resource| StructuredOutput::Resource(Box::new(resource)))
                    .map_err(|error| map_boundary(error, OperationFailure::Resource))
            }
            StructuredOperation::Hbs(operation) => {
                let result = match operation.as_ref() {
                    HbsOperation::ReadPreservation => {
                        hbs_storage::preservation::read_with(&mut sources, db, "structured", id)
                    }
                    HbsOperation::SavePreservation(checkpoint) => {
                        hbs_storage::preservation::save_with(
                            &mut sources,
                            db,
                            "structured",
                            id,
                            checkpoint.clone(),
                        )
                    }
                    HbsOperation::Commit(request) => {
                        hbs_storage::commit_with(&mut sources, db, "structured", id, request)
                    }
                };
                result
                    .map(StructuredOutput::Hbs)
                    .map_err(|error| map_boundary(error, OperationFailure::Hbs))
            }
            StructuredOperation::Review(input) => structured_resources::review_with(
                &mut sources,
                db,
                input.clone(),
                ReviewRoute::Native,
            )
            .map(|resource| StructuredOutput::Resource(Box::new(resource)))
            .map_err(|error| map_boundary(error, OperationFailure::Resource)),
            StructuredOperation::History(input) => structured_resources::restore_history_with(
                &mut sources,
                db,
                input.clone(),
                ReviewRoute::Native,
            )
            .map(|resource| StructuredOutput::Resource(Box::new(resource)))
            .map_err(|error| map_boundary(error, OperationFailure::Resource)),
            StructuredOperation::Write(input) | StructuredOperation::TextWrite(input) => {
                structured_resources::write_with(&mut sources, db, input.clone())
                    .map(|result| StructuredOutput::Write(Box::new(result)))
                    .map_err(|error| map_boundary(error, OperationFailure::Resource))
            }
        };
        let recovery = !db.is_autocommit()
            || result.as_ref().is_err_and(|error| match error {
                BoundaryError::Failure(error) => error.needs_recovery(),
                BoundaryError::Pause(_) => false,
            });
        let result = if recovery {
            match result {
                Err(BoundaryError::Failure(mut error)) => {
                    error.outcome = HbsOutcome::Unknown;
                    Err(BoundaryError::Failure(error))
                }
                _ => Err(BoundaryError::Failure(failure(
                    "MODEL_STORAGE_ERROR：结构化事务未确认结算。".into(),
                    "execution",
                    HbsOutcome::Unknown,
                    FailureCause::Original,
                ))),
            }
        } else {
            result
        };
        (result, recovery)
    });
    let mut value = match result {
        Ok(value) => value,
        Err(error) => return Ok((sources, Err(error))),
    };
    // History retains its original draft-read and notify with the DB guard
    // alive; review and HBS commit retain their original drop-before-notify.
    if let (StructuredOperation::History(_), StructuredOutput::Resource(resource)) =
        (operation, &mut value)
    {
        structured_resources::attach_latest_draft(&db, resource).map_err(|error| {
            let cause = match &error {
                structured_resources::Error::Database(error) => FailureCause::SourceDatabase(
                    Box::new(SafeDatabaseError::sql("postcommit-read", error)),
                ),
                _ => FailureCause::Original,
            };
            OperationError::Resource(BoundaryFailure {
                failure: Failure { error, cause },
                phase: "postcommit-read",
                outcome: HbsOutcome::Committed,
                rollback_error: None,
            })
        })?;
        store.notify_structured_resource_changed(&resource.id, resource.paper_id.as_deref());
        return Ok((sources, Ok(value)));
    }
    let paper = if matches!(operation, StructuredOperation::Hbs(value) if matches!(value.as_ref(), HbsOperation::Commit(_)))
    {
        let StructuredOutput::Hbs(result) = &value else {
            return Err(OperationError::unknown());
        };
        let outcome = if result["type"] == "receipt" {
            HbsOutcome::Committed
        } else {
            HbsOutcome::NotStarted
        };
        Some(
            structured_resources::get(&db, id)
                .map_err(|error| {
                    let message = format!("MODEL_STORAGE_ERROR：{}：{error}", error.command_code());
                    let cause = match error {
                        structured_resources::Error::Database(error) => {
                            FailureCause::SourceDatabase(Box::new(SafeDatabaseError::sql(
                                "postcommit-read",
                                &error,
                            )))
                        }
                        _ => FailureCause::Original,
                    };
                    operation_error(failure(message, "postcommit-read", outcome, cause))
                })?
                .paper_id,
        )
    } else {
        None
    };
    if matches!(operation, StructuredOperation::TextWrite(_)) {
        return Ok((sources, Ok(value)));
    }
    drop(db);
    match &value {
        StructuredOutput::Resource(resource) => {
            store.notify_structured_resource_changed(&resource.id, resource.paper_id.as_deref())
        }
        StructuredOutput::Write(result) => store.notify_structured_resource_changed(
            &result.resource.id,
            result.resource.paper_id.as_deref(),
        ),
        StructuredOutput::Hbs(result) if result["type"] == "receipt" => {
            if let Some(paper) = paper {
                store.notify_structured_resource_changed(id, paper.as_deref());
            }
        }
        _ => {}
    }
    Ok((sources, Ok(value)))
}

#[cfg(test)]
type HbsAttempt = (RuntimeHbsSources, Result<Value, BoundaryError<HbsNeed>>);
#[cfg(test)]
pub(super) async fn attempt(
    store: AppStore,
    id: String,
    operation: Arc<HbsOperation>,
    sources: RuntimeHbsSources,
) -> Result<HbsAttempt, HbsOperationError> {
    let (sources, result) = attempt_operation(
        store,
        id,
        Arc::new(StructuredOperation::Hbs(operation)),
        sources,
    )
    .await
    .map_err(OperationError::into_hbs)?;
    let result = match result {
        Ok(StructuredOutput::Hbs(value)) => Ok(value),
        Ok(StructuredOutput::Resource(_) | StructuredOutput::Write(_)) => {
            return Err(HbsOperationError::unknown());
        }
        Err(BoundaryError::Pause(pause)) => Err(BoundaryError::Pause(pause)),
        Err(BoundaryError::Failure(error)) => match operation_error(error).into_hbs() {
            HbsOperationError::Boundary(error) => Err(BoundaryError::Failure(error)),
            error => return Err(error),
        },
    };
    Ok((sources, result))
}
