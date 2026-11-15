//! Original metadata statement and settlement order over fixed package resources.
use super::{outcome::*, resources::MetadataResources};
use crate::{
    data_interface::{DatabaseGuard, guarded_transaction, plugin_resources::ColumnCodec},
    runtime_plugins::{
        literature_catalog::{
            data::{CatalogResources, capture_items},
            save::{
                self, CommittedCatalogRows, SaveCause, SaveFailure, SaveOutcome, SaveResources,
            },
        },
        native::SafeDatabaseError,
    },
};
use research_workbench_native_sdk::{
    catalog_save::{CatalogSaveOutcome, CatalogSaveRejection},
    data::DataValue,
    metadata::*,
};
use rusqlite::{Connection, Transaction};

mod derivation;
mod operations;
mod patch;
mod transaction;

type Result<T> = std::result::Result<T, MetadataFailure>;
type StepResult<T> = std::result::Result<T, (&'static str, MetadataProblem)>;

#[derive(Debug)]
pub(crate) enum MetadataExecution {
    Item(CommittedCatalogRows),
    Done,
}

pub(crate) fn apply(
    guard: &mut DatabaseGuard<'_>,
    resources: &MetadataResources,
    catalog: &CatalogResources,
    save: &SaveResources,
    preparation: &MetadataPreparationV1,
) -> Result<MetadataExecution> {
    if resources.package_key() != catalog.package_key()
        || resources.package_key() != save.package_key()
    {
        return Err(MetadataFailure::new(
            "scope",
            MetadataProblem::Binding,
            MetadataPhaseOutcome::NotStarted,
        ));
    }
    if let MetadataPreparationV1::FullApply { plan } = preparation {
        return full(guard, resources, catalog, save, plan);
    }
    guarded_transaction::run(guard, |db| {
        let result = match preparation {
            MetadataPreparationV1::Update { plan } => {
                operations::update(db, resources, catalog, plan)
            }
            MetadataPreparationV1::Confirm { outcome } => {
                operations::confirm(db, resources, catalog, outcome)
            }
            MetadataPreparationV1::FieldsApply { outcome } => {
                operations::fields(db, resources, catalog, outcome)
            }
            MetadataPreparationV1::Reject { plan } => operations::reject(db, resources, plan),
            MetadataPreparationV1::FullApply { .. } => {
                unreachable!("handled before guarded dispatch")
            }
        };
        let unknown = result
            .as_ref()
            .is_err_and(|error| error.outcome == MetadataPhaseOutcome::Unknown);
        (result, unknown)
    })
}

fn full(
    guard: &mut DatabaseGuard<'_>,
    resources: &MetadataResources,
    catalog: &CatalogResources,
    save_resources: &SaveResources,
    plan: &MetadataFullApplyPlanV1,
) -> Result<MetadataExecution> {
    let save_plan = match &plan.save {
        CatalogSaveOutcome::Planned { plan } => plan,
        CatalogSaveOutcome::Rejected { reason } => {
            // The original save opens its Deferred transaction before validation.
            save::probe_begin(guard)
                .map_err(|error| first_save_failure(error, plan.input.header.item_id.as_deref()))?;
            let rejection = match reason {
                CatalogSaveRejection::UnsupportedType { item_type } => {
                    MetadataRejectionV1::UnsupportedType {
                        item_type: item_type.clone(),
                    }
                }
                CatalogSaveRejection::EmptyTitle => MetadataRejectionV1::EmptyTitle {},
            };
            let mut error = MetadataFailure::new(
                "first-save-validation",
                MetadataProblem::Domain(rejection),
                MetadataPhaseOutcome::RolledBack,
            );
            error.first_save = Some(SaveOutcome::RolledBack);
            error.item_id = plan.input.header.item_id.clone();
            return Err(error);
        }
    };
    let first = save::persist(guard, save_resources, catalog, save_plan)
        .map_err(|error| first_save_failure(error, plan.input.header.item_id.as_deref()))?;
    let first_outcome = SaveOutcome::Committed {
        item_id: first.item_id.clone(),
    };
    if first
        .items
        .first()
        .is_none_or(|item| item.main.rows.is_empty())
    {
        let mut error = MetadataFailure::new(
            "first-save-read",
            MetadataProblem::Domain(MetadataRejectionV1::NotFound {}),
            MetadataPhaseOutcome::Committed,
        );
        error.item_id = Some(first.item_id);
        error.first_save = Some(first_outcome);
        return Err(error);
    }
    let item_id = first.item_id;
    guarded_transaction::run(guard, |db| {
        let result =
            operations::full_second(db, resources, catalog, plan, &item_id).map_err(|mut error| {
                error.first_save = Some(first_outcome);
                error.item_id = Some(item_id.clone());
                error
            });
        let unknown = result
            .as_ref()
            .is_err_and(|error| error.outcome == MetadataPhaseOutcome::Unknown);
        (result, unknown)
    })
}

fn first_save_failure(error: SaveFailure, known_item_id: Option<&str>) -> MetadataFailure {
    let outcome = match &error.outcome {
        SaveOutcome::NotStarted => MetadataPhaseOutcome::NotStarted,
        SaveOutcome::RolledBack => MetadataPhaseOutcome::RolledBack,
        SaveOutcome::Committed { .. } => MetadataPhaseOutcome::Committed,
        SaveOutcome::Unknown => MetadataPhaseOutcome::Unknown,
    };
    let item_id = match &error.outcome {
        SaveOutcome::Committed { item_id } => Some(item_id.clone()),
        _ => known_item_id.map(str::to_owned),
    };
    let problem = match error.cause {
        SaveCause::Data(error) => MetadataProblem::Data(error),
        SaveCause::Database(error) => MetadataProblem::Database(error),
    };
    let mut failure = MetadataFailure::new("first-save", problem, outcome);
    failure.first_save = Some(error.outcome);
    failure.item_id = item_id;
    failure.rollback_error = error.rollback_error;
    failure
}

fn current(
    db: &Connection,
    catalog: &CatalogResources,
    id: &str,
    outcome: MetadataPhaseOutcome,
) -> Result<CommittedCatalogRows> {
    let items = capture_items(db, catalog, &[id.into()]).map_err(|cause| {
        MetadataFailure::new("item-read", MetadataProblem::Data(cause), outcome.clone())
    })?;
    if items.first().is_none_or(|item| item.main.rows.is_empty()) {
        return Err(MetadataFailure::new(
            "item-read",
            MetadataProblem::Domain(MetadataRejectionV1::NotFound {}),
            outcome,
        ));
    }
    Ok(CommittedCatalogRows {
        item_id: id.into(),
        items,
    })
}

fn text(value: &str) -> DataValue {
    DataValue::Text(value.into())
}
fn optional_text(value: Option<&str>) -> DataValue {
    value.map_or(DataValue::Null, text)
}
fn year(value: Option<i32>) -> DataValue {
    value.map_or(DataValue::Null, |value| {
        DataValue::Integer(i64::from(value))
    })
}
fn uuid() -> DataValue {
    text(&uuid::Uuid::new_v4().to_string())
}

fn write(
    tx: &Transaction<'_>,
    resources: &MetadataResources,
    key: &'static str,
    arguments: Vec<DataValue>,
) -> StepResult<u64> {
    resources
        .write_in(tx, key, arguments)
        .map_err(|error| (key, MetadataProblem::Data(error)))
}
