//! Native reads and their lock waits run outside the event loop.
use rusqlite::Connection;

use crate::storage::{AppStore, StorageError};

impl AppStore {
    /// Acquire the existing connection on a blocking worker, including lock wait.
    /// The closure retains the original query, connection and error semantics.
    pub(crate) async fn query<T, E>(
        &self,
        query: impl FnOnce(&Connection) -> Result<T, E> + Send + 'static,
    ) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<StorageError> + Send + 'static,
    {
        self.blocking(move |store| {
            let db = store.connection()?;
            query(&db)
        })
        .await
    }

    /// Own a finite native operation, acquiring its existing locks on the worker.
    /// Awaiting this does not change the operation's transaction or lock order.
    pub(crate) async fn blocking<T, E>(
        &self,
        action: impl FnOnce(&AppStore) -> Result<T, E> + Send + 'static,
    ) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<StorageError> + Send + 'static,
    {
        let store = self.clone();
        tauri::async_runtime::spawn_blocking(move || action(&store))
            .await
            .map_err(|error| E::from(StorageError::QueryWorker(error.to_string())))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{future::Future, sync::mpsc, task::Poll, time::Duration};

    #[tokio::test]
    async fn contended_query_yields_and_reads_committed_state_on_worker() {
        let root = tempfile::tempdir().unwrap();
        let store = AppStore::for_test(
            Connection::open_in_memory().unwrap(),
            root.path().join("objects"),
        );
        store
            .connection()
            .unwrap()
            .execute_batch("create table sample(value text); insert into sample values('before');")
            .unwrap();
        let owner = store.clone();
        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let holding = std::thread::spawn(move || {
            let db = owner.connection().unwrap();
            locked_tx.send(()).unwrap();
            // A regressed inline lock wait fails rather than hanging the suite.
            let released = release_rx.recv_timeout(Duration::from_secs(3)).is_ok();
            db.execute("update sample set value='after'", []).unwrap();
            released
        });
        locked_rx.recv().unwrap();
        let caller = std::thread::current().id();
        let mut query = Box::pin(store.query(move |db| -> Result<_, StorageError> {
            assert_ne!(std::thread::current().id(), caller);
            Ok(db.query_row("select value from sample", [], |row| {
                row.get::<_, String>(0)
            })?)
        }));
        std::future::poll_fn(|cx| {
            assert!(
                query.as_mut().poll(cx).is_pending(),
                "database contention must yield to the caller"
            );
            Poll::Ready(())
        })
        .await;
        tokio::task::yield_now().await;
        release_tx.send(()).unwrap();
        assert_eq!(query.await.unwrap(), "after");
        assert!(holding.join().unwrap());
    }

    #[tokio::test]
    async fn file_operation_waits_for_bibliography_lock_off_caller_thread() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("main.tex");
        std::fs::write(&file, "before").unwrap();
        let store = AppStore::for_test(
            Connection::open_in_memory().unwrap(),
            root.path().join("objects"),
        );
        let owner = store.clone();
        let (locked_tx, locked_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let changed = file.clone();
        let holding = std::thread::spawn(move || {
            let _guard = owner.bibliography_io().unwrap();
            locked_tx.send(()).unwrap();
            let released = release_rx.recv_timeout(Duration::from_secs(3)).is_ok();
            std::fs::write(changed, "committed").unwrap();
            released
        });
        locked_rx.recv().unwrap();
        let caller = std::thread::current().id();
        let mut operation = Box::pin(store.blocking(move |store| -> Result<_, StorageError> {
            assert_ne!(std::thread::current().id(), caller);
            let _guard = store.bibliography_io()?;
            let _db = store.connection()?;
            Ok(std::fs::read_to_string(file)?)
        }));
        std::future::poll_fn(|cx| {
            assert!(operation.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        release_tx.send(()).unwrap();
        assert_eq!(operation.await.unwrap(), "committed");
        assert!(holding.join().unwrap());
    }

    #[tokio::test]
    async fn query_keeps_original_errors_and_reports_worker_failure() {
        let root = tempfile::tempdir().unwrap();
        let store = AppStore::for_test(
            Connection::open_in_memory().unwrap(),
            root.path().join("objects"),
        );
        let missing = store
            .query(|db| -> Result<(), StorageError> {
                db.execute("select * from missing_table", [])?;
                Ok(())
            })
            .await
            .unwrap_err();
        assert!(matches!(missing, StorageError::Database(_)));
        let failed = store
            .query(|_| -> Result<(), StorageError> { panic!("synthetic worker failure") })
            .await
            .unwrap_err();
        assert!(
            matches!(&failed, StorageError::QueryWorker(cause) if cause.contains("synthetic worker failure"))
        );
        let cause = "synthetic worker failure";
        let errors = [
            serde_json::to_value(crate::notes::NotesCommandError::from(
                StorageError::QueryWorker(cause.into()),
            ))
            .unwrap(),
            serde_json::to_value(crate::structured_resources::CommandError::from(
                StorageError::QueryWorker(cause.into()),
            ))
            .unwrap(),
        ];
        for error in errors {
            assert_eq!(error["code"], "db_query_worker_failed");
            assert!(error["message"].as_str().unwrap().contains(cause));
        }
    }
}
