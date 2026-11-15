use super::*;

pub(crate) struct Mode {
    fresh: bool,
    reopen: bool,
}

impl Mode {
    pub(crate) fn parse(arguments: &[String]) -> Self {
        Self {
            fresh: arguments.len() == 2 && arguments[1] == "--doi",
            reopen: arguments.len() == 3 && arguments[1] == "--reopen-doi",
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
        if self.selected() && (!cfg!(debug_assertions) || !cfg!(feature = "development-data")) {
            return Err("DOI acceptance requires a debug --no-default-features --features development-data build".into());
        }
        Ok(())
    }
    pub(crate) fn prefix(&self) -> Option<&'static str> {
        self.selected().then_some("l02-real-doi-")
    }
    pub(crate) fn marker(&self) -> Option<(&'static str, &'static str)> {
        self.selected()
            .then_some(("synthetic-doi.json", "doi-check"))
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
            receipt["status"] = json!("passed-doi-native-core-only");
            receipt["scope"] = json!(
                "L02 DOI backend workflow with synthetic loopback HTTP and real Native/SQL; not live DOI, frontend, desktop or complete C01/C03 acceptance"
            );
        }
    }
    pub(crate) fn write_marker(&self, directory: &Path) -> Result<(), Error> {
        if self.fresh {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("synthetic-doi.json"))?
                .write_all(&serde_json::to_vec(
                    &json!({"kind":"doi-check","version":1}),
                )?)?;
        }
        Ok(())
    }
}
