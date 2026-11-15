//! Writing error mapping for the shared immutable object staging owner.
use super::*;
use crate::agent::workspace::{ProposalObjects, object_staging};
use research_agent_core::workspace::FileVersion;

fn writing_error(error: object_staging::Error) -> Error {
    match error {
        object_staging::Error::Lease(error) => Error::Lease(error),
        object_staging::Error::Storage => Error::Storage,
        object_staging::Error::RecoveryRequired => Error::RecoveryRequired,
    }
}
pub(super) fn stage(
    store: &AppStore,
    objects: &ProposalObjects,
    version: &FileVersion,
    bytes: &[u8],
) -> Result<(), Error> {
    object_staging::stage(store, objects, version, bytes).map_err(writing_error)
}
pub(super) fn recover(store: &AppStore) -> Result<(usize, usize), Error> {
    object_staging::recover(store).map_err(writing_error)
}
