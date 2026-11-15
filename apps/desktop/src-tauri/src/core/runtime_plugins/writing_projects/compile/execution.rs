//! The original snapshot, fixed command and artifact publication stages.
use super::{cache, process, repository, resources::Sql, snapshot, tools};
use crate::{agent::writing_error::WritingError, storage::AppStore};
use repository::{CacheArtifact, CompileCompletion, StoredPdf};
use research_workbench_native_sdk::writing_compile::{
    CompileOptions, Completion, EngineArgument, Flag, LatexToolchainInfo, RunStatus,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use uuid::Uuid;

const MAX_PDF_BYTES: u64 = 256 * 1024 * 1024;
const MAX_SYNCTEX_BYTES: u64 = 256 * 1024 * 1024;
const COMPILE_TIMEOUT: Duration = Duration::from_secs(120);

pub(crate) struct Prepared {
    run_id: String,
    manuscript_id: String,
    root_path: String,
    snapshot: PathBuf,
    output: PathBuf,
    temporary: PathBuf,
    toolchain_home: PathBuf,
    latexmk: PathBuf,
    input_hash: String,
}

pub(crate) struct Launched {
    outcome: process::ProcessOutcome,
    pub(crate) raw_output: String,
}

impl Launched {
    pub(crate) fn state(&self) -> research_workbench_native_sdk::writing_compile::ProcessState {
        self.outcome.state
    }
    pub(crate) fn successful(&self) -> bool {
        self.outcome.status.success()
    }
}

pub(crate) fn prepare(
    store: &AppStore,
    sql: &Sql,
    run_id: &str,
    manuscript_id: &str,
    root_path: &str,
    toolchain: &LatexToolchainInfo,
) -> Result<Prepared, WritingError> {
    cache::cleanup_manuscript_runs(sql, store, manuscript_id, Some(run_id))?;
    {
        let db = store.connection()?;
        repository::mark_running(sql, &db, run_id)?;
    }
    let run_root = store
        .writing_cache_directory()
        .join(manuscript_id)
        .join(run_id);
    let snapshot = run_root.join("snapshot");
    let output = run_root.join("output");
    let temporary = run_root.join("tmp");
    fs::create_dir_all(&snapshot)?;
    fs::create_dir_all(&output)?;
    fs::create_dir_all(&temporary)?;
    let toolchain_home = store.writing_cache_directory().join("toolchain-home");
    fs::create_dir_all(&toolchain_home)?;
    let source = crate::scoped_text_files::legacy::manuscript_source_directory(
        store.manuscript_directory(),
        manuscript_id,
    )?;
    let input_hash = snapshot::snapshot_source(&source, &snapshot)?;
    let root = snapshot.join(crate::scoped_text_files::legacy::validate_relative_path(
        root_path,
    )?);
    if !root.is_file() {
        return Err(WritingError::FileNotFound);
    }
    let latexmk = tools::executable(toolchain, "latexmk")?;
    Ok(Prepared {
        run_id: run_id.into(),
        manuscript_id: manuscript_id.into(),
        root_path: root_path.into(),
        snapshot,
        output,
        temporary,
        toolchain_home,
        latexmk,
        input_hash,
    })
}

pub(crate) fn launch(
    prepared: &Prepared,
    options: &CompileOptions,
    toolchain: &LatexToolchainInfo,
    engine: EngineArgument,
    flags: &[Flag],
    cancellation: Arc<AtomicBool>,
) -> Result<Launched, WritingError> {
    let mut command = Command::new(&prepared.latexmk);
    command
        .current_dir(&prepared.snapshot)
        .env_clear()
        .env("PATH", tools::compile_path(toolchain))
        .env("HOME", &prepared.toolchain_home)
        .env("TMPDIR", &prepared.temporary)
        .arg("-norc")
        .arg(engine.argument())
        .arg("-interaction=nonstopmode")
        .arg("-file-line-error")
        .arg("-synctex=1")
        .arg("-latexoption=-no-shell-escape")
        .arg(format!("-outdir={}", prepared.output.to_string_lossy()))
        .arg(&prepared.root_path);
    if let Ok(language) = std::env::var("LANG") {
        command.env("LANG", language);
    }
    command.args(flags.iter().copied().map(Flag::argument));
    let outcome = process::run(&mut command, cancellation, COMPILE_TIMEOUT)?;
    let raw_output = format!(
        "Compile options: {}\n{}",
        serde_json::to_string(options)?,
        combine_output(&outcome.stdout, &outcome.stderr)
    );
    Ok(Launched {
        outcome,
        raw_output,
    })
}

pub(crate) fn finish(
    store: &AppStore,
    sql: &Sql,
    prepared: &Prepared,
    launched: &Launched,
    completion: &Completion,
    toolchain: &LatexToolchainInfo,
) -> Result<(), WritingError> {
    let status = completion.status.stored_value();
    let log_path = prepared.output.join("compile.log");
    fs::write(&log_path, launched.raw_output.as_bytes())?;
    let stem = Path::new(&prepared.root_path)
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or(WritingError::InvalidPath)?;
    let pdf_path = prepared.output.join(format!("{stem}.pdf"));
    let pdf = if completion.status == RunStatus::Succeeded
        || (completion.status == RunStatus::Failed && recoverable_pdf(&pdf_path)?)
    {
        Some(store_pdf(store, &pdf_path)?)
    } else {
        None
    };
    let synctex_path = prepared.output.join(format!("{stem}.synctex.gz"));
    let cache_prefix = format!("{}/{}/output", prepared.manuscript_id, prepared.run_id);
    let log_relative = format!("{cache_prefix}/compile.log");
    let synctex_relative = format!("{cache_prefix}/{stem}.synctex.gz");
    let log_size = fs::metadata(&log_path)?.len();
    let synctex = synctex_path
        .is_file()
        .then(|| fs::metadata(&synctex_path).map(|value| value.len()))
        .transpose()?;
    if synctex.is_some_and(|size| size > MAX_SYNCTEX_BYTES) {
        return Err(WritingError::Process(
            "LaTeX 生成的 SyncTeX 数据超过 256 MiB 限制".to_owned(),
        ));
    }
    let toolchain_json = serde_json::to_string(toolchain)?;
    let stored = CompileCompletion {
        status,
        input_hash: &prepared.input_hash,
        toolchain_json: &toolchain_json,
        raw_output: &launched.raw_output,
        exit_code: launched.outcome.status.code(),
        diagnostics: &completion.diagnostics,
        pdf,
        log: Some(CacheArtifact {
            relative_path: log_relative,
            byte_size: log_size,
        }),
        synctex: synctex.map(|byte_size| CacheArtifact {
            relative_path: synctex_relative,
            byte_size,
        }),
    };
    {
        let mut db = store.connection()?;
        repository::finish_run(sql, &mut db, &prepared.run_id, stored)?;
    }
    if let Err(error) =
        cache::cleanup_manuscript_runs(sql, store, &prepared.manuscript_id, Some(&prepared.run_id))
    {
        eprintln!(
            "failed to clean LaTeX build cache after run {}: {error}",
            prepared.run_id
        );
    }
    Ok(())
}

pub(crate) fn recoverable_pdf(source: &Path) -> Result<bool, WritingError> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() || metadata.len() > MAX_PDF_BYTES {
        return Ok(false);
    }
    let bytes = fs::read(source)?;
    // Reject empty/truncated output; PDF.js performs the full document parsing.
    Ok(bytes.starts_with(b"%PDF-")
        && bytes[bytes.len().saturating_sub(1024)..]
            .windows(5)
            .any(|window| window == b"%%EOF"))
}

fn store_pdf(store: &AppStore, source: &Path) -> Result<StoredPdf, WritingError> {
    if fs::metadata(source)?.len() > MAX_PDF_BYTES {
        return Err(WritingError::Process(
            "LaTeX 生成的 PDF 超过 256 MiB 限制".to_owned(),
        ));
    }
    let bytes = fs::read(source)?;
    if !bytes.starts_with(b"%PDF-") {
        return Err(WritingError::Process(
            "latexmk 成功但未生成有效 PDF".to_owned(),
        ));
    }
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let object_relative_path = format!("{}/{}/{}.pdf", &sha256[0..2], &sha256[2..4], sha256);
    let destination = store.content_directory().join(&object_relative_path);
    if !destination.exists() {
        let parent = destination.parent().ok_or(WritingError::InvalidPath)?;
        fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(".writing-{}.tmp", Uuid::new_v4()));
        let write_result = (|| {
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &destination)
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&temporary);
            if !destination.exists() {
                return Err(error.into());
            }
        }
    }
    Ok(StoredPdf {
        sha256,
        object_relative_path,
        byte_size: bytes.len() as u64,
    })
}

fn combine_output(stdout: &str, stderr: &str) -> String {
    match (stdout.is_empty(), stderr.is_empty()) {
        (false, false) => format!("{stdout}\n--- stderr ---\n{stderr}"),
        (false, true) => stdout.to_owned(),
        (true, false) => stderr.to_owned(),
        (true, true) => String::new(),
    }
}
