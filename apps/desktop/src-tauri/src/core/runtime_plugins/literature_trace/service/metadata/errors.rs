use super::*;
use crate::data_interface::DataError;
use crate::runtime_plugins::native::SafeDatabaseError;

pub(super) fn owner(cause: DataError) -> Box<NativeHostError> {
    let problem = match cause {
        DataError::Sql(error) => {
            MetadataProblem::Database(Box::new(SafeDatabaseError::sql("owner", &error)))
        }
        DataError::Unavailable => MetadataProblem::Owner("DATA_RUNTIME_UNAVAILABLE"),
        DataError::Pair(_) => MetadataProblem::Owner("DATA_RUNTIME_PAIR_INVALID"),
        DataError::Log(_) => MetadataProblem::Owner("DATA_RUNTIME_LOG"),
        DataError::Admission(_) => MetadataProblem::Owner("DATA_RUNTIME_ADMISSION"),
    };
    MetadataFailure::new("owner", problem, MetadataPhaseOutcome::NotStarted).native()
}

pub(super) fn unknown(phase: &'static str, cause: &'static str) -> Box<NativeHostError> {
    MetadataFailure::new(
        phase,
        MetadataProblem::Owner(cause),
        MetadataPhaseOutcome::Unknown,
    )
    .native()
}

#[cfg(unix)]
pub(super) fn bind(host: &NativeHost, mut error: Box<NativeHostError>) -> Box<NativeHostError> {
    if let Some((instance, request)) = host.request_identity() {
        error.instance_id = Some(instance);
        error.request_id = Some(request);
    }
    error
}

pub(super) fn committed(
    mut error: Box<NativeHostError>,
    id: &str,
    full: bool,
) -> Box<NativeHostError> {
    let mut failure = MetadataFailure::new(
        "projection",
        MetadataProblem::Owner("METADATA_RESULT_UNAVAILABLE"),
        MetadataPhaseOutcome::Committed,
    );
    failure.item_id = Some(id.into());
    if full {
        failure.first_save = Some(
            crate::runtime_plugins::literature_catalog::save::SaveOutcome::Committed {
                item_id: id.into(),
            },
        );
    }
    error.literature_metadata = Some(Box::new(failure));
    error
}
