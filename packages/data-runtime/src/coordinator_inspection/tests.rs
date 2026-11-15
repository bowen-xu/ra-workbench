use super::*;
use std::{io, sync::TryLockError};

struct EmptyProjector(Uuid);
impl Projector for EmptyProjector {
    type Command = u64;
    type Receipt = u64;
    type Error = io::Error;
    fn operation_id(command: &u64) -> Uuid {
        Uuid::from_u128(u128::from(*command))
    }
    fn state(&self) -> Result<ProjectionState, io::Error> {
        Ok(ProjectionState {
            epoch: self.0,
            materialized_position: 0,
        })
    }
    fn lookup(&self, _: Uuid) -> Result<Option<OperationReceipt<u64>>, io::Error> {
        panic!("inspection cannot look up receipts")
    }
    fn validate(&self, _: &u64, _: &[Accepted<u64, u64>]) -> Result<Admission<u64>, io::Error> {
        panic!("inspection cannot validate mutations")
    }
    fn apply_batch(&self, _: &[Accepted<u64, u64>]) -> Result<(), io::Error> {
        panic!("inspection cannot materialize")
    }
}
fn owner(path: &std::path::Path) -> Coordinator<EmptyProjector> {
    let epoch = Uuid::new_v4();
    Coordinator::open(
        RaftLog::open(path, Some(epoch)).unwrap(),
        EmptyProjector(epoch),
        CoordinatorConfig::default(),
    )
    .unwrap()
}

#[test]
fn inspection_uses_the_exact_existing_admission_mutex() {
    let directory = tempfile::tempdir().unwrap();
    let owner = owner(directory.path());
    let guard = owner.inspection_guard().unwrap();
    let before = guard.state.log.head().unwrap();
    assert!(matches!(
        owner.state.try_lock(),
        Err(TryLockError::WouldBlock)
    ));
    assert_eq!(guard.snapshot().epoch, before.epoch);
    assert_eq!(guard.state.log.head().unwrap(), before);
    drop(guard);
    assert!(owner.state.try_lock().is_ok());
}

#[test]
fn poison_is_recovery_required_for_inspection_without_bypassing_or_clearing_it() {
    let directory = tempfile::tempdir().unwrap();
    let owner = owner(directory.path());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = owner.inspection_guard().unwrap();
            panic!("synthetic status reader panic");
        }))
        .is_err()
    );
    assert!(matches!(
        owner.inspection_guard(),
        Err(CoordinatorError::RecoveryRequired)
    ));
    assert!(matches!(
        owner.write(1, WriteOptions::default()),
        Err(CoordinatorError::RecoveryRequired)
    ));
    assert!(matches!(
        owner.query::<()>(|_, _| panic!("poisoned owner cannot query")),
        Err(CoordinatorError::RecoveryRequired)
    ));
    assert!(owner.state.is_poisoned());
}
