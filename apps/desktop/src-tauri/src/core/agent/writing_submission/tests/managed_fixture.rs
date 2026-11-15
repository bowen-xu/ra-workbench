//! Typed mutations exercise a real owner; the deployed 123 fixture stays intact.
use super::*;

pub(in crate::agent::writing_submission) fn managed_fixture(bound: bool) -> Fixture {
    managed_fixture_at(bound, tempfile::tempdir().unwrap())
}
pub(in crate::agent::writing_submission) fn managed_fixture_at(
    bound: bool,
    root: tempfile::TempDir,
) -> Fixture {
    let mut fixture = fixture_at(bound, root);
    let mut sql = Connection::open_in_memory().unwrap();
    {
        let legacy = fixture.store.connection().unwrap();
        rusqlite::backup::Backup::new(&legacy, &mut sql)
            .unwrap()
            .run_to_completion(256, std::time::Duration::ZERO, None)
            .unwrap();
    }
    crate::storage::migrate(&mut sql).unwrap();
    let database =
        crate::data_interface::Database::open(sql, &fixture.root.path().join("data-log")).unwrap();
    fixture.store = AppStore::for_test_database(database, fixture.root.path().join("data/objects"));
    fixture.models = AgentModelResolver::from_app_store(
        &fixture.store,
        JsonCredentialStore::from_path(fixture.root.path().join("credentials.json")),
    );
    fixture
}
