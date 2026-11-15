//! Development acceptance for the same Core catalog/service used by Tauri.
//! Uses synthetic paths supplied by the developer; never accesses the app DB.
use crate::runtime_plugins::{RuntimePluginCatalog, native::NativePreviewService};
use std::{path::PathBuf, sync::Arc};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err(
            "usage: native-preview-check <controlled-package-directory> <source-directory-or-zip>"
                .into(),
        );
    }
    let catalog = Arc::new(
        RuntimePluginCatalog::new(Some(PathBuf::from(&args[0]))).with_native_writing_preview(true),
    );
    let listing = catalog.discover()?;
    if listing.packages.len() != 1 {
        return Err("native-preview-check requires exactly the one frozen B00 package".into());
    }
    let key = listing.packages[0].package_key.clone();
    let service = NativePreviewService::new(catalog);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let result = service.preview(key, args[1].clone()).await;
        let cleanup = service.shutdown().await;
        match result {
            Ok(preview) => println!("{}", serde_json::json!({"status":"passed-native-preview-only","preview":preview,"shutdown":cleanup.as_ref().map(|_| "passed").unwrap_or("failed")})),
            Err(error) => { println!("{}", serde_json::to_string(&error)?); return Err(Box::<dyn std::error::Error>::from(error)); }
        }
        cleanup?;
        Ok(())
    })
}
