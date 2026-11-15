//! Only the new metadata mode's composition; old CLI modes stay in their owner.
use super::*;

pub(crate) struct Mode {
    fresh: bool,
    reopen: bool,
}

impl Mode {
    pub(crate) fn parse(arguments: &[String]) -> Self {
        Self {
            fresh: arguments.len() == 2 && arguments[1] == "--metadata",
            reopen: arguments.len() == 3 && arguments[1] == "--reopen-metadata",
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
        self.selected().then_some("l02-real-metadata-")
    }

    pub(crate) fn marker(&self) -> Option<(&'static str, &'static str)> {
        self.selected()
            .then_some(("synthetic-metadata.json", "metadata-check"))
    }

    pub(crate) async fn run(
        &self,
        store: &AppStore,
        service: &Arc<LiteratureTraceService>,
        key: &str,
        directory: &Path,
    ) -> Result<Value, Error> {
        if self.reopen {
            super::reopen(store, service, key, directory).await
        } else {
            super::run(store, service, key).await
        }
    }

    pub(crate) fn annotate(&self, receipt: &mut Value) {
        if self.selected() {
            receipt["status"] = json!("passed-metadata-native-core-only");
            receipt["scope"] = json!(
                "L02 five metadata backend consumers only; no preview provider, frontend or complete C01/C03 acceptance"
            );
        }
    }

    pub(crate) fn write_marker(&self, directory: &Path) -> Result<(), Error> {
        if self.fresh {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("synthetic-metadata.json"))?
                .write_all(&serde_json::to_vec(
                    &json!({"kind":"metadata-check","version":1}),
                )?)?;
        }
        Ok(())
    }
}
