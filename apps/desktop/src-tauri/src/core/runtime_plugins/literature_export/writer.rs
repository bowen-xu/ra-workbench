use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use uuid::Uuid;

use super::manager::ResearchExportManager;
use super::{ResearchExportError, ResearchExportReport};
use research_workbench_native_sdk::research_export::{
    MAX_EXPORT_BYTES, PreparedResearchExportV1 as ResearchExportBundle,
};

pub(crate) fn write_export(
    target_directory: &str,
    bundle: &ResearchExportBundle,
    manager: &ResearchExportManager,
) -> Result<ResearchExportReport, ResearchExportError> {
    manager.check_cancelled()?;
    let target =
        fs::canonicalize(target_directory).map_err(|_| ResearchExportError::InvalidTarget)?;
    if !target.is_dir() {
        return Err(ResearchExportError::InvalidTarget);
    }
    let manifest = bundle.manifest_json.as_bytes();
    let markdown = bundle.markdown.as_bytes();
    if manifest.len().saturating_add(markdown.len()) > MAX_EXPORT_BYTES {
        return Err(ResearchExportError::TooLarge);
    }
    let folder_name = export_folder_name(&bundle.exported_at);
    let destination = target.join(&folder_name);
    if destination.exists() {
        return Err(ResearchExportError::DestinationExists);
    }
    let temporary = target.join(format!(".research-export-{}.tmp", Uuid::new_v4()));
    fs::create_dir(&temporary)?;
    let result = (|| -> Result<(), ResearchExportError> {
        write_new(&temporary.join("manifest.json"), manifest)?;
        manager.check_cancelled()?;
        write_new(&temporary.join("research-knowledge.md"), markdown)?;
        manager.check_cancelled()?;
        fs::rename(&temporary, &destination)?;
        Ok(())
    })();
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }
    Ok(ResearchExportReport {
        folder_name,
        file_count: 2,
        object_count: bundle.object_count,
        source_count: bundle.source_count,
    })
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), ResearchExportError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn export_folder_name(exported_at: &str) -> String {
    let timestamp = exported_at
        .chars()
        .map(|character| match character {
            ':' | '.' => '-',
            value => value,
        })
        .collect::<String>();
    format!("research-assistant-knowledge-{timestamp}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn publication_failure_after_both_writes_removes_the_original_temporary_directory() {
        let directory = tempfile::tempdir().unwrap();
        // A nested fixture timestamp forces rename into a missing parent after both writes.
        let bundle = ResearchExportBundle {
            exported_at: "missing/target".into(),
            manifest_json: "{}".into(),
            markdown: "Synthetic export".into(),
            object_count: 0,
            source_count: 0,
        };
        let error = write_export(
            directory.path().to_str().unwrap(),
            &bundle,
            &ResearchExportManager::default(),
        )
        .unwrap_err();
        assert!(matches!(error, ResearchExportError::Io(_)));
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }
}
