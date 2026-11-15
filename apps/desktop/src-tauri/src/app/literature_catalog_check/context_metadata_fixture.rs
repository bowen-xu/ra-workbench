//! Fresh synthetic business data for the ordinary metadata/group GUI acceptance.
//! No Agent admission or tool invocation occurs during this preparation.
use super::*;
use crate::{
    runtime_plugins::literature_trace::binding::NoteTraceBinding, storage::AppStoreSources,
};
use research_workbench_native_sdk::context_metadata::{CAPABILITY_ID, CAPABILITY_VERSION};
use std::{fs, io::Write, path::PathBuf};
#[path = "context_metadata_fixture/seed.rs"]
mod seed;

const FLAG: &str = "--context-metadata-fixture";
const PREFIX: &str = "l09-context-group-fixture-";
pub(super) struct Request<'a> {
    package_root: &'a str,
    package_key: &'a str,
}
pub(super) fn argument(arguments: &[String]) -> Result<Option<Request<'_>>, Error> {
    match arguments {
        [package_root, mode, package_key] if mode == FLAG && valid_key(package_key) =>
            Ok(Some(Request { package_root, package_key })),
        _ if arguments.iter().any(|value| value.starts_with(FLAG)) => Err(
            "usage: literature-catalog-check <controlled-package-root> --context-metadata-fixture <exact-package-key>; fresh preparation only".into(),
        ),
        _ => Ok(None),
    }
}
fn valid_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
pub(super) fn run(request: Request<'_>) -> Result<(), Error> {
    if !cfg!(debug_assertions) || !cfg!(feature = "development-data") {
        return Err("context metadata fixture requires debug --no-default-features --features development-data".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let temporary = root.join(".tmp");
    if !fs::symlink_metadata(&temporary)?.file_type().is_dir()
        || temporary.canonicalize()?.parent() != Some(root.as_path())
    {
        return Err("context metadata fixture requires the real repository .tmp directory".into());
    }
    let package_root = PathBuf::from(request.package_root);
    if !fs::symlink_metadata(&package_root)?.file_type().is_dir() {
        return Err("context metadata fixture package root must be a real directory".into());
    }
    let package_root = package_root.canonicalize()?;
    if !package_root.starts_with(&temporary) || package_root == temporary {
        return Err(
            "context metadata fixture accepts an explicit package root below repository .tmp"
                .into(),
        );
    }
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(package_root.clone())));
    let listing = catalog.discover()?;
    if listing.packages.len() != 1
        || listing.packages[0].package_key != request.package_key
        || listing.packages[0].manifest["id"] != "literature_library"
    {
        return Err(
            "context metadata fixture requires exactly the selected literature package".into(),
        );
    }
    if !listing.packages[0].manifest["provides"]
        .as_array()
        .is_some_and(|values| {
            values
                .iter()
                .any(|value| value["id"] == CAPABILITY_ID && value["version"] == CAPABILITY_VERSION)
        })
    {
        return Err("selected fixture package does not declare context metadata projection".into());
    }
    // Declarations identify the intended fixture. Actual execution admission
    // remains in the existing catalog-save and structured-write service calls.
    let parent = temporary.join("runtime-plugins");
    if !parent.exists() {
        fs::create_dir(&parent)?;
    }
    if !fs::symlink_metadata(&parent)?.file_type().is_dir()
        || parent.canonicalize()?.parent() != Some(temporary.as_path())
    {
        return Err(
            "context metadata fixture parent escaped repository temporary ownership".into(),
        );
    }
    let directory = tempfile::Builder::new()
        .prefix(PREFIX)
        .tempdir_in(&parent)?
        .keep()
        .canonicalize()?;
    if directory.parent() != Some(parent.as_path()) {
        return Err("fresh context metadata fixture escaped its fixed parent".into());
    }
    let store_path = directory.join("store");
    let configuration = directory.join("configuration");
    fs::create_dir(&configuration)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let service = Arc::new(LiteratureTraceService::new(catalog));
    let key = request.package_key.to_owned();
    let store = AppStore::open_with_paths(
        AppStorePaths {
            data_directory: store_path.clone(),
            agent_configuration_directory: configuration.clone(),
        },
        AppStorePorts::with_events(
            RuntimeEvents::new(|_, _| Ok(())),
            AppStoreSources::default(),
        ),
    )?
    .with_native_note_trace(NoteTraceBinding {
        service: Arc::clone(&service),
        package_key: key.clone(),
    });
    if store_path.canonicalize()?.parent() != Some(directory.as_path())
        || configuration.canonicalize()?.parent() != Some(directory.as_path())
    {
        return Err("fresh fixture Store/configuration identity differs".into());
    }
    let result = runtime.block_on(async {
        let prepared = seed::prepare(&store, &service, &key).await;
        let closed = service.shutdown().await;
        match (prepared, closed) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(error)) => Err(error.into()),
            (Err(error), Err(close)) => Err(format!(
                "context fixture preparation failed: {error}; Native shutdown failed: {close}"
            )
            .into()),
        }
    });
    let database = store.shared_agent_connection();
    drop(store);
    let released = Arc::strong_count(&service) == 1;
    drop(service);
    let closed = database.shutdown();
    drop(database);
    drop(runtime);
    closed?;
    if !released {
        return Err("context metadata fixture retained an operation owner".into());
    }
    let mut receipt = result?;
    receipt["kind"] = json!("context-metadata-gui-business-fixture");
    receipt["version"] = json!(1);
    receipt["status"] = json!("prepared-business-fixture-only");
    receipt["packageKey"] = json!(key);
    receipt["packageRoot"] = json!(package_root);
    receipt["store"] = json!(store_path);
    receipt["configuration"] = json!(configuration);
    receipt["shutdown"] = json!({"native":"passed","database":"passed","ownersReleased":true});
    receipt["scope"] = json!(
        "Fresh seeded business fixture through original catalog and Core Note/structured writers. No Agent admission, model history, tool proof, parser/PDF, GUI, failure recovery or cold acceptance. This mode accepts no existing Store and has no reopen/reseed branch."
    );
    let path = directory.join("fixture.json");
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)?;
    output.write_all(&serde_json::to_vec_pretty(&receipt)?)?;
    output.write_all(b"\n")?;
    output.sync_all()?;
    println!(
        "{}",
        json!({"receipt":path,"status":receipt["status"],"store":store_path,"run":directory})
    );
    Ok(())
}

#[cfg(test)]
#[path = "context_metadata_fixture/tests.rs"]
mod tests;
