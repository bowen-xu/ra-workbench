//! Development-only real-package acceptance of Core's structured source tool.
use super::*;
use crate::{
    agent::{repository::submissions::SqliteSubmissionStore, service::AgentSessionService},
    runtime_plugins::literature_context::PaperContextResources,
    runtime_plugins::literature_trace::binding::NoteTraceBinding,
};
use research_agent_core::submission::{SubmissionRecord, SubmissionStore};
use serde::{Deserialize, Serialize};
use std::{fs, io::Write, path::PathBuf};

#[path = "structured_source_check/consumer.rs"]
mod consumer;
#[path = "structured_source_check/fixture.rs"]
mod fixture;
#[path = "structured_source_check/readback.rs"]
mod readback;
#[path = "structured_source_check/safety.rs"]
mod safety;

const PREFIX: &str = "l09-real-structured-source-";
const MARKER: &str = "synthetic-structured-source.json";
const KIND: &str = "structured-source-tool-check";
const FRESH: &str = "--structured-source";
const REOPEN: &str = "--reopen-structured-source";

pub(super) struct Request<'a> {
    package: &'a str,
    reopen: Option<&'a str>,
}

pub(super) fn argument(arguments: &[String]) -> Result<Option<Request<'_>>, Error> {
    match arguments {
        [package, mode] if mode == FRESH => Ok(Some(Request { package, reopen: None })),
        [package, mode, directory] if mode == REOPEN => {
            Ok(Some(Request { package, reopen: Some(directory) }))
        }
        _ if arguments.iter().any(|value| value.starts_with(FRESH) || value.starts_with(REOPEN)) => {
            Err("usage: literature-catalog-check <controlled-package-directory> --structured-source | --reopen-structured-source <synthetic-run>".into())
        }
        _ => Ok(None),
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Saved {
    kind: String,
    version: u32,
    package_key: String,
    seed: fixture::Seed,
    binding: SubmissionBinding,
    state: readback::State,
    output: Value,
}

#[derive(Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SubmissionBinding {
    conversation_id: uuid::Uuid,
    request_id: uuid::Uuid,
    turn_id: uuid::Uuid,
    context_snapshot_id: uuid::Uuid,
    input_sha256: String,
}

impl SubmissionBinding {
    fn capture(record: &SubmissionRecord) -> Result<Self, Error> {
        Ok(Self {
            conversation_id: record.conversation_id,
            request_id: record.request_id,
            turn_id: record.turn_id,
            context_snapshot_id: record.context_snapshot_id,
            input_sha256: format!("{:x}", Sha256::digest(serde_json::to_vec(&record.input)?)),
        })
    }
}

pub(super) fn run(request: Request<'_>) -> Result<(), Error> {
    if !cfg!(debug_assertions) || !cfg!(feature = "development-data") {
        return Err("structured source acceptance requires debug --no-default-features --features development-data".into());
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()?;
    let parent = root.join(".tmp/runtime-plugins").canonicalize()?;
    if !parent.starts_with(root.join(".tmp")) {
        return Err("structured source acceptance requires repository temporary ownership".into());
    }
    let catalog = Arc::new(RuntimePluginCatalog::new(Some(request.package.into())));
    let listing = catalog.discover()?;
    if listing.packages.len() != 1 || listing.packages[0].manifest["id"] != "literature_library" {
        return Err(
            "structured source acceptance requires exactly one independent literature package"
                .into(),
        );
    }
    let key = listing.packages[0].package_key.clone();
    let directory = directory(&parent, request.reopen)?;
    let saved = request
        .reopen
        .map(|_| read_saved(&directory, &key))
        .transpose()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let resources = PaperContextResources::load(&catalog, &listing.packages[0])?;
    let service = Arc::new(LiteratureTraceService::new(catalog));
    if request.reopen.is_some() {
        if directory.join("store").canonicalize()?.parent() != Some(directory.as_path())
            || !directory.join("store/research-assistant.sqlite3").is_file()
            || directory.join("configuration").canonicalize()?.parent() != Some(directory.as_path())
        {
            return Err(
                "cold source acceptance requires its existing owned store/configuration".into(),
            );
        }
    } else {
        fs::create_dir_all(directory.join("configuration"))?;
    }
    let store = AppStore::open_with_paths(
        AppStorePaths {
            data_directory: directory.join("store"),
            agent_configuration_directory: directory.join("configuration"),
        },
        AppStorePorts::with_events(RuntimeEvents::new(|_, _| Ok(())), core_sources::sources()),
    )?
    .with_native_note_trace(NoteTraceBinding {
        service: Arc::clone(&service),
        package_key: key.clone(),
    });
    if store.require_paper_reading_source().is_ok() {
        return Err("structured source acceptance unexpectedly has a static reading owner".into());
    }
    let agents = Arc::new(AgentSessionService::from_app_store(&store)?);
    let result: Result<Saved, Error> = runtime.block_on(async {
        let result = exercise(
            &store, &service, &agents, &resources, &key, &directory, saved,
        )
        .await;
        let agent_close = agents.shutdown().await;
        let native_close = service.shutdown().await;
        match (result, agent_close, native_close) {
            (Ok(value), Ok(()), Ok(())) => Ok(value),
            (result, agent, native) => Err(format!(
                "structured source acceptance/close failed: operation={}; agent={}; native={}",
                result
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "passed".into()),
                agent
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "passed".into()),
                native
                    .err()
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "passed".into()),
            )
            .into()),
        }
    });
    let database = store.shared_agent_connection();
    drop(agents);
    drop(store);
    let native_released = Arc::strong_count(&service) == 1;
    drop(service);
    let database_close = database.shutdown();
    drop(database);
    drop(runtime);
    database_close?;
    if !native_released {
        return Err("structured source acceptance retained a Native operation owner".into());
    }
    let saved = result?;
    let (filename, status) = if request.reopen.is_some() {
        (
            "structured-source-reopen.json",
            "passed-real-package-structured-source-cold",
        )
    } else {
        save_new(&directory.join(MARKER), &serde_json::to_value(&saved)?)?;
        (
            "structured-source-fresh.json",
            "passed-real-package-structured-source-fresh",
        )
    };
    let receipt = json!({"status":status,"packageKey":key,"store":directory.join("store"),
        "conversationId":saved.binding.conversation_id,"requestId":saved.binding.request_id,
        "turnId":saved.binding.turn_id,"snapshotId":saved.state.snapshot.id,
        "checks":{"actualToolRegistration":true,"actualNativePackage":true,"authorizedPageOnly":true,
            "ungrantedAttachmentRejected":true,"repeatedReadStable":true,"coreNoteAndCasPreserved":true,
            "agentShutdown":true,"nativeShutdown":true,"databaseShutdown":true,"ownersReleased":true},
        "scope":"Synthetic real PDF/CAS and seeded Docling cache through the production tool registration. Provider/parser execution and Agent-loop history require their separate acceptance.",
        "fixture":MARKER});
    save_new(&directory.join(filename), &receipt)?;
    println!(
        "{}",
        json!({"receipt":directory.join(filename),"status":status,"run":directory})
    );
    Ok(())
}

fn directory(parent: &Path, reopen: Option<&str>) -> Result<PathBuf, Error> {
    if let Some(directory) = reopen {
        let directory = PathBuf::from(directory).canonicalize()?;
        if directory.parent() != Some(parent)
            || !directory
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(PREFIX))
        {
            return Err(
                "structured source reopen requires its exact synthetic repository run".into(),
            );
        }
        safety::validate_tree(&directory)?;
        return Ok(directory);
    }
    Ok(tempfile::Builder::new()
        .prefix(PREFIX)
        .tempdir_in(parent)?
        .keep()
        .canonicalize()?)
}

fn read_saved(directory: &Path, key: &str) -> Result<Saved, Error> {
    let saved: Saved = serde_json::from_slice(&fs::read(directory.join(MARKER))?)?;
    if saved.kind != KIND || saved.version != 1 || saved.package_key != key {
        return Err("structured source synthetic marker/package identity differs".into());
    }
    Ok(saved)
}

async fn exercise(
    store: &AppStore,
    service: &Arc<LiteratureTraceService>,
    agents: &Arc<AgentSessionService>,
    resources: &PaperContextResources,
    key: &str,
    directory: &Path,
    saved: Option<Saved>,
) -> Result<Saved, Error> {
    if let Some(saved) = saved {
        agents
            .open(
                research_agent_core::conversation::ConversationId::from_uuid(
                    saved.binding.conversation_id,
                ),
            )
            .await?;
        let record = SqliteSubmissionStore::for_app(store)
            .get(saved.binding.conversation_id, saved.binding.request_id)
            .await?;
        if SubmissionBinding::capture(&record)? != saved.binding {
            return Err("cold submission identity or frozen input differs".into());
        }
        readback::verify_prefix(store, &saved.seed)?;
        let result = consumer::exercise(store, resources, &saved.seed, &record).await?;
        if result.state != saved.state || result.output != saved.output {
            return Err("cold structured source read differs from its saved snapshot/proof".into());
        }
        return Ok(saved);
    }
    let seed = fixture::prepare(store, service, key, directory).await?;
    let record = consumer::submission(store, agents, &seed).await?;
    let result = consumer::exercise(store, resources, &seed, &record).await?;
    Ok(Saved {
        kind: KIND.into(),
        version: 1,
        package_key: key.into(),
        seed,
        binding: SubmissionBinding::capture(&record)?,
        state: result.state,
        output: result.output,
    })
}

fn save_new(path: &Path, value: &Value) -> Result<(), Error> {
    let mut output = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    output.write_all(&serde_json::to_vec_pretty(value)?)?;
    output.sync_all()?;
    Ok(())
}
