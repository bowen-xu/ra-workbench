//! Cancellable document exports. Resource packing and file I/O run on blocking
//! workers; native PDF rendering is isolated from the application's UI owner.
#[path = "jobs.rs"]
mod jobs;
#[path = "pack.rs"]
mod pack;

use crate::notes::NoteExportAssetReader;
use jobs::ExportJob;
pub use jobs::NoteExportJobs;
use pack::ExportInput;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
};
use tauri::State;
use tokio::io::AsyncReadExt;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Pdf,
    Html,
}
impl Format {
    fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Html => "html",
        }
    }
}

#[derive(Serialize)]
pub struct Capabilities {
    pdf: bool,
    html: bool,
}

fn renderer_path() -> Result<PathBuf, String> {
    let executable =
        std::env::current_exe().map_err(|error| format!("NOTE_PDF_RENDERER_PATH：{error}"))?;
    Ok(executable
        .parent()
        .ok_or("NOTE_PDF_RENDERER_PATH：缺少程序目录。")?
        .join("note-pdf-worker"))
}

fn platform_supports_pdf() -> bool {
    #[cfg(target_os = "macos")]
    {
        objc2_foundation::NSProcessInfo::processInfo()
            .operatingSystemVersion()
            .majorVersion
            >= 11
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[tauri::command]
pub fn note_document_export_capabilities() -> Capabilities {
    Capabilities {
        pdf: platform_supports_pdf() && renderer_path().is_ok_and(|path| path.is_file()),
        html: true,
    }
}

#[tauri::command]
pub fn begin_note_document_export(
    jobs: State<'_, NoteExportJobs>,
    job_id: String,
) -> Result<(), String> {
    jobs.begin(&job_id)
}
#[tauri::command]
pub fn cancel_note_document_export(
    jobs: State<'_, NoteExportJobs>,
    job_id: String,
) -> Result<bool, String> {
    jobs.cancel(&job_id)
}
#[tauri::command]
pub fn release_note_document_export(
    jobs: State<'_, NoteExportJobs>,
    job_id: String,
) -> Result<(), String> {
    jobs.release(&job_id)
}

async fn render_native(input: &Path, output: &Path, job: &ExportJob) -> Result<(), String> {
    if !platform_supports_pdf() {
        return Err("NOTE_PDF_PLATFORM_UNAVAILABLE：此平台尚无原生 PDF 导出。".into());
    }
    job.check()?;
    let executable = renderer_path()?;
    if !executable.is_file() {
        return Err("NOTE_PDF_RENDERER_MISSING：应用缺少原生 PDF 渲染组件。".into());
    }
    let mut child = tokio::process::Command::new(executable)
        .arg(input)
        .arg(output)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("NOTE_PDF_RENDERER_START：{error}"))?;
    wait_renderer(&mut child, job).await
}

async fn wait_renderer(child: &mut tokio::process::Child, job: &ExportJob) -> Result<(), String> {
    let stderr = child.stderr.take();
    let mut diagnostics = tokio::spawn(async move {
        let mut output = Vec::new();
        if let Some(stderr) = stderr {
            let _ = stderr.take(8192).read_to_end(&mut output).await;
        }
        output
    });
    let status = tokio::select! {
        biased;
        _=job.cancelled.notified()=>{
            let _=child.kill().await; let _=child.wait().await;
            diagnostics.abort();let _=diagnostics.await;
            return Err("NOTE_EXPORT_CANCELLED：导出已停止。".into());
        },
        status=child.wait()=>status.map_err(|error|format!("NOTE_PDF_RENDERER_WAIT：{error}"))?,
    };
    let diagnostics =
        match tokio::time::timeout(std::time::Duration::from_secs(2), &mut diagnostics).await {
            Ok(result) => {
                result.map_err(|error| format!("NOTE_PDF_RENDERER_DIAGNOSTICS：{error}"))?
            }
            Err(_) => {
                diagnostics.abort();
                Vec::new()
            }
        };
    job.check()?;
    if !status.success() {
        // Framework stderr may contain page internals. Only our stable, safe
        // diagnostic lines cross the IPC boundary; never forward raw stderr.
        let safe = String::from_utf8_lossy(&diagnostics)
            .lines()
            .find(|line| line.starts_with("NOTE_"))
            .map(str::to_owned);
        return Err(safe
            .unwrap_or_else(|| format!("NOTE_PDF_RENDER_FAILED：原生排版进程退出（{status}）。")));
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stopping_a_renderer_kills_and_reaps_the_real_child() {
        let jobs = NoteExportJobs::default();
        let id = uuid::Uuid::new_v4().to_string();
        jobs.begin(&id).unwrap();
        let job = jobs.get(&id).unwrap();
        let mut child = tokio::process::Command::new("/bin/sleep")
            .arg("60")
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        assert!(jobs.cancel(&id).unwrap());
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            wait_renderer(&mut child, &job),
        )
        .await
        .unwrap();
        assert!(result.unwrap_err().contains("NOTE_EXPORT_CANCELLED"));
        assert!(child.try_wait().unwrap().is_some());
    }
}

#[tauri::command]
pub async fn run_note_document_export(
    app: tauri::AppHandle,
    assets: State<'_, NoteExportAssetReader>,
    jobs: State<'_, NoteExportJobs>,
    job_id: String,
    destination: String,
    format: Format,
    input: ExportInput,
) -> Result<(), String> {
    let job = jobs.get(&job_id)?;
    job.claim()?;
    let result = run_export(app, assets.inner().clone(), job, destination, format, input).await;
    // Terminal cleanup belongs to the native task, even if its originating
    // WebView/editor is closed before the invoke promise is observed.
    let released = jobs.release(&job_id);
    match (result, released) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

async fn run_export(
    app: tauri::AppHandle,
    assets: NoteExportAssetReader,
    job: Arc<ExportJob>,
    destination: String,
    format: Format,
    input: ExportInput,
) -> Result<(), String> {
    if matches!(format, Format::Pdf) && !platform_supports_pdf() {
        return Err("NOTE_PDF_PLATFORM_UNAVAILABLE：此平台尚无原生 PDF 导出。".into());
    }
    let destination = PathBuf::from(destination);
    if !destination.is_absolute()
        || destination
            .extension()
            .and_then(|value| value.to_str())
            .is_none_or(|extension| !extension.eq_ignore_ascii_case(format.extension()))
    {
        return Err("NOTE_EXPORT_DESTINATION_INVALID：保存路径或扩展名无效。".into());
    }
    let directory =
        tempfile::tempdir().map_err(|error| format!("NOTE_EXPORT_TEMPORARY_IO：{error}"))?;
    let html = directory.path().join("document.html");
    let pdf = directory.path().join("document.pdf");
    let pack_job = Arc::clone(&job);
    let html_output = html.clone();
    tokio::task::spawn_blocking(move || pack::pack(&app, &assets, input, &html_output, &pack_job))
        .await
        .map_err(|error| format!("NOTE_EXPORT_PACK_WORKER：{error}"))??;
    job.check()?;
    let source = match format {
        Format::Html => html,
        Format::Pdf => {
            render_native(&html, &pdf, &job).await?;
            pdf
        }
    };
    tokio::task::spawn_blocking(move || {
        pack::publish(&source, &destination, format.extension(), &job)
    })
    .await
    .map_err(|error| format!("NOTE_EXPORT_PUBLISH_WORKER：{error}"))??;
    // Keep the private directory alive through publication and child teardown.
    drop(directory);
    Ok(())
}
