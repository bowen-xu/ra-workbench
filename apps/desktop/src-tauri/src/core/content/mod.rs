mod pdf_objects;
pub(crate) use pdf_objects::{PdfImportSource, PdfObjectError, PdfObjectStore};
mod stored_read;
pub(crate) use stored_read::{StoredObjectRead, StoredObjectReadError};
#[path = "asset_name.rs"]
mod asset_name;
#[path = "commands.rs"]
mod commands;
#[path = "error.rs"]
mod error;
#[path = "gc.rs"]
mod gc;
#[path = "gc_manager.rs"]
mod gc_manager;
#[path = "model.rs"]
mod model;
#[path = "references.rs"]
mod references;

use crate::storage::AppStore;
use std::time::Duration;

pub(crate) use asset_name::note_image_display_name;
pub use commands::{
    cancel_content_gc, get_content_gc_status, get_content_storage_report, scan_content_storage,
    start_content_gc,
};
pub(crate) use gc_manager::ContentGcManager;

impl AppStore {
    /// Internal repository capability; no lock, connection or transaction is acquired.
    pub(crate) fn pdf_objects(&self) -> PdfObjectStore<'_> {
        PdfObjectStore::for_directory(self.content_directory())
    }
}

pub(crate) fn schedule_startup_maintenance(store: AppStore, manager: ContentGcManager) {
    tauri::async_runtime::spawn(async move {
        loop {
            let task_store = store.clone();
            let task_manager = manager.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || {
                run_scheduled_maintenance_pass(&task_store, &task_manager);
            })
            .await;
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
        }
    });
}

fn run_scheduled_maintenance_pass(store: &AppStore, manager: &ContentGcManager) {
    if manager.begin().is_ok() {
        let _ = manager.set_phase("recovery");
        let _ = gc::recover_quarantine(store, manager);
        manager.finish();
    }

    let (needs_initial_scan, is_daily_cleanup_due) = store
        .connection()
        .and_then(|connection| {
            connection
                .query_row(
                    "select
                           (backfill_status <> 'complete' or last_mark_at is null),
                           (last_mark_at is not null
                            and julianday(last_mark_at) <= julianday('now', '-1 day')
                            and (last_sweep_at is null
                              or julianday(last_sweep_at) <= julianday('now', '-1 day')))
                         from content_gc_state where id = 1",
                    [],
                    |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?)),
                )
                .map_err(Into::into)
        })
        .unwrap_or((false, false));

    if !needs_initial_scan && !is_daily_cleanup_due {
        return;
    }
    if manager.begin().is_err() {
        return;
    }
    if is_daily_cleanup_due {
        let _ = gc::run_gc(store, manager, false);
    } else {
        // The first background pass only indexes references and candidates.
        // It deliberately gives upgraded users time to inspect and pin
        // history before any automatic retention pruning can run.
        let _ = gc::run_scan(store, manager);
    }
    manager.finish();
}
