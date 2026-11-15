//! Actual independent-package services and synthetic Core SQL/log acceptance.
use super::*;
use crate::runtime_plugins::literature_attachments::{AttachmentRequest, AttachmentResult};
use research_workbench_native_sdk::{
    attachments::{AttachmentRemovalResult, PageText, SaveReadingStateInputV1},
    catalog::CatalogAttachment,
    duplicate_review::F64BitsV1,
    import_jobs::{ImportJobsRequestV1, ImportJobsResultV1, ImportedJob},
    pdf_import::PdfImportCandidate,
};

#[path = "fixture.rs"]
mod fixture;
#[path = "identity.rs"]
mod identity;
#[path = "pdf.rs"]
mod pdf;
#[path = "readback.rs"]
mod readback;
#[path = "scenarios.rs"]
mod scenarios;

pub(crate) struct Mode {
    fresh: bool,
    reopen: bool,
}

impl Mode {
    pub(crate) fn parse(arguments: &[String]) -> Self {
        Self {
            fresh: arguments.len() == 2 && arguments[1] == "--attachments",
            reopen: arguments.len() == 3 && arguments[1] == "--reopen-attachments",
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
    pub(crate) fn prefix(&self) -> Option<&'static str> {
        self.selected().then_some("l03-real-attachments-")
    }
    pub(crate) fn marker(&self) -> Option<(&'static str, &'static str)> {
        self.selected()
            .then_some(("synthetic-attachments.json", "attachments-check"))
    }
    pub(crate) fn check_features(&self) -> Result<(), Error> {
        if self.selected() && (!cfg!(debug_assertions) || !cfg!(feature = "development-data")) {
            return Err("attachment acceptance requires debug --no-default-features --features development-data".into());
        }
        Ok(())
    }
    pub(crate) fn annotate(&self, receipt: &mut Value) {
        if self.selected() {
            receipt["status"] = json!("passed-attachments-native-core-only");
            receipt["scope"] = json!(
                "L03 attachment backend (Reveal unverified without AppHandle), adjacent PDF worker/shared jobs, fixed SQL/log/files and Core Note; no PDF.js/UI/desktop or complete L03/C01-C09 acceptance"
            );
        }
    }
    pub(crate) fn write_marker(&self, directory: &Path) -> Result<(), Error> {
        if self.fresh {
            use std::io::Write;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(directory.join("synthetic-attachments.json"))?
                .write_all(&serde_json::to_vec(
                    &json!({"kind":"attachments-check","version":1}),
                )?)?;
        }
        Ok(())
    }
    pub(crate) async fn run(
        &self,
        store: &AppStore,
        service: &Arc<LiteratureTraceService>,
        key: &str,
        directory: &Path,
    ) -> Result<Value, Error> {
        if self.reopen {
            reopen(store, key, directory)
        } else {
            run(store, service, key, directory).await
        }
    }
}

async fn call(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    request: AttachmentRequest,
) -> Result<AttachmentResult, Error> {
    Ok(service
        .attachments(store.clone(), key.into(), request)
        .await?)
}
async fn attach(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    item: &str,
    path: &Path,
) -> Result<CatalogAttachment, Error> {
    match call(
        store,
        service,
        key,
        AttachmentRequest::Attach {
            item_id: item.into(),
            path: path.to_str().ok_or("synthetic UTF-8 source path")?.into(),
        },
    )
    .await?
    {
        AttachmentResult::Attachment(value) => Ok(value),
        _ => Err("attachment result kind".into()),
    }
}
async fn remove(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    item: &str,
    id: &str,
) -> Result<AttachmentRemovalResult, Error> {
    match call(
        store,
        service,
        key,
        AttachmentRequest::Remove {
            item_id: item.into(),
            attachment_id: id.into(),
        },
    )
    .await?
    {
        AttachmentResult::Removed(value) => Ok(value),
        _ => Err("remove result kind".into()),
    }
}
async fn done(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    request: AttachmentRequest,
) -> Result<(), Error> {
    match call(store, service, key, request).await? {
        AttachmentResult::Done => Ok(()),
        _ => Err("void result kind".into()),
    }
}
async fn run(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    key: &str,
    directory: &Path,
) -> Result<Value, Error> {
    let note = fixture::seed(store)?;
    let initial = readback::snapshot(store)?;
    let mut receipt = scenarios::run(store, service, key, directory).await?;
    if fixture::note(store)? != note {
        return Err("attachment workflow changed the full Core Note document".into());
    }
    let actual = readback::snapshot(store)?;
    if readback::protected(&actual)? != readback::protected(&initial)? {
        return Err("attachment workflow changed original Core Note projections".into());
    }
    let files = readback::files(store)?;
    if !store.connection()?.is_autocommit() {
        return Err("attachment workflow left an open transaction".into());
    }
    receipt["noteId"] = json!(fixture::NOTE);
    receipt["noteDocument"] = note;
    receipt["sqlSnapshot"] = actual;
    receipt["files"] = files;
    receipt["coreOnly"] = json!(true);
    receipt["providerCalls"] = json!(0);
    receipt["successfulCallsExposeRequestIdentity"] = json!(false);
    receipt["snapshotScope"] = json!(
        "24 explicit SQL projections plus complete Core Note and exact stored-object bytes/hash; not whole database or database-file bytes"
    );
    Ok(receipt)
}
fn reopen(store: &AppStore, key: &str, directory: &Path) -> Result<Value, Error> {
    let bytes = std::fs::read(directory.join("receipt.json"))?;
    let original: Value = serde_json::from_slice(&bytes)?;
    if original["status"] != "passed-attachments-native-core-only"
        || original["packageKey"] != key
        || original["shutdown"] != "passed"
        || original["coreOnly"] != true
        || original["noteId"] != fixture::NOTE
    {
        return Err(
            "cold attachment check requires its original successful synthetic receipt".into(),
        );
    }
    let actual = readback::snapshot(store)?;
    let files = readback::files(store)?;
    let note = fixture::note(store)?;
    if actual != original["sqlSnapshot"]
        || files != original["files"]
        || note != original["noteDocument"]
    {
        return Err(
            "cold SQL/log reopen changed attachment/Note identity, metadata or stored bytes".into(),
        );
    }
    if readback::snapshot(store)? != actual || !store.connection()?.is_autocommit() {
        return Err("cold read-only verification changed fixed projections".into());
    }
    Ok(
        json!({"coreOnly":true,"noteId":fixture::NOTE,"noteDocument":note,"sqlSnapshot":actual,"files":files,
        "originalReceiptSha256":format!("{:x}",Sha256::digest(&bytes)),"originalReceiptUnchanged":std::fs::read(directory.join("receipt.json"))?==bytes,
        "coldReopenNoSeedOrRequestReplay":true,"noNativeRequestDuringReopen":true,"noAttachmentOrImportServiceCallDuringReopen":true,
        "snapshotScope":original["snapshotScope"],"revealVerified":false}),
    )
}
