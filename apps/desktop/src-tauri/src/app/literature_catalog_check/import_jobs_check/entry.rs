//! Only the new import-jobs mode's composition; old CLI modes stay in their owner.
use super::*;

pub(crate) struct Mode {
    fresh: bool,
    reopen: bool,
}

impl Mode {
    pub(crate) fn before_open(&self, directory: &Path) -> Result<Option<String>, Error> {
        if !self.reopen {
            return Ok(None);
        }
        // The top-level mode has already verified this exact synthetic run marker.
        let path = directory
            .join("store/research-assistant.sqlite3")
            .canonicalize()?;
        if !path.starts_with(directory) {
            return Err("cold import clock must use the marked synthetic run's database".into());
        }
        let db = rusqlite::Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        Ok(Some(db.query_row(
            "select strftime('%Y-%m-%dT%H:%M:%fZ','now')",
            [],
            |row| row.get(0),
        )?))
    }
    pub(crate) fn parse(arguments: &[String]) -> Self {
        Self {
            fresh: arguments.len() == 2 && arguments[1] == "--import-jobs",
            reopen: arguments.len() == 3 && arguments[1] == "--reopen-import-jobs",
        }
    }

    pub(crate) fn fresh(&self) -> bool {
        self.fresh
    }

    pub(crate) fn reopen(&self) -> bool {
        self.reopen
    }

    pub(crate) fn selected(&self) -> bool {
        self.fresh || self.reopen
    }

    pub(crate) fn check_features(&self) -> Result<(), Error> {
        Ok(())
    }

    pub(crate) fn prefix(&self) -> Option<&'static str> {
        self.selected().then_some("l02-real-import-jobs-")
    }

    pub(crate) fn marker(&self) -> Option<(&'static str, &'static str)> {
        self.selected()
            .then_some(("synthetic-import-jobs.json", "import-jobs-check"))
    }

    pub(crate) async fn run(
        &self,
        store: &AppStore,
        service: &Arc<LiteratureTraceService>,
        key: &str,
        directory: &Path,
        recovery_start: Option<&str>,
    ) -> Result<Value, Error> {
        if self.reopen {
            super::reopen(store, service, key, directory, recovery_start).await
        } else {
            super::run(store, service, key).await
        }
    }

    pub(crate) fn annotate(&self, receipt: &mut Value) {
        if self.selected() {
            receipt["status"] = json!("passed-import-jobs-native-core-only");
            receipt["scope"] = json!(
                "L02 BibTeX import and shared import-job get/list/cancel only; no PDF/DOI execution, frontend or complete C01/C03 acceptance"
            );
        }
    }

    pub(crate) fn write_marker(&self, directory: &Path) -> Result<(), Error> {
        if self.fresh {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("synthetic-import-jobs.json"))?
                .write_all(&serde_json::to_vec(
                    &json!({"kind":"import-jobs-check","version":1}),
                )?)?;
        }
        Ok(())
    }
}
