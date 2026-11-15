use super::jobs::ExportJob;
use crate::notes::NoteExportAssets;
use serde::Deserialize;
use std::{
    fs::File,
    io::{BufWriter, Read, Write},
    path::Path,
};

pub const MAX_EXPORT_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportInput {
    pub template: String,
    pub resources: Vec<Resource>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Resource {
    Font {
        token: String,
        name: String,
    },
    Image {
        token: String,
        #[serde(rename = "assetId")]
        asset_id: String,
    },
}

fn io(error: std::io::Error) -> String {
    format!("NOTE_EXPORT_IO：{error}")
}

impl Resource {
    fn token(&self) -> &str {
        match self {
            Self::Font { token, .. } | Self::Image { token, .. } => token,
        }
    }
}

fn font_name_valid(name: &str) -> bool {
    name.len() <= 128
        && (name == "ResearchAssistantSans-Regular.ttf"
            || name == "ResearchAssistantSans-Bold.ttf"
            || (name.starts_with("KaTeX_")
                && name.ends_with(".ttf")
                && !name.contains("..")
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"_- .".contains(&byte))
                && !name.contains(' ')))
}

pub fn pack(
    app: &tauri::AppHandle,
    assets: &impl NoteExportAssets,
    input: ExportInput,
    path: &Path,
    job: &ExportJob,
) -> Result<(), String> {
    if input.template.len() > 16 * 1024 * 1024
        || !input.template.starts_with("<!doctype html>")
        || input.resources.len() > 2048
    {
        return Err("NOTE_EXPORT_INPUT_INVALID：导出输入无效或超出大小限制。".into());
    }
    let mut ranges = Vec::new();
    for resource in &input.resources {
        let token = resource.token();
        if !token.starts_with("NOTE_EXPORT_RESOURCE_")
            || token.len() > 128
            || !token
                .bytes()
                .all(|value| value.is_ascii_alphanumeric() || value == b'_')
        {
            return Err("NOTE_EXPORT_RESOURCE_INVALID：资源占位标识无效。".into());
        }
        let mut matches = input.template.match_indices(token);
        let start = matches
            .next()
            .ok_or("NOTE_EXPORT_RESOURCE_INVALID：资源占位标识缺失。")?
            .0;
        if matches.next().is_some() {
            return Err("NOTE_EXPORT_RESOURCE_INVALID：资源占位标识不唯一。".into());
        }
        ranges.push((start, start + token.len(), resource));
    }
    ranges.sort_by_key(|range| range.0);
    let mut writer = BufWriter::new(File::create(path).map_err(io)?);
    let mut cursor = 0;
    let mut written = input.template.len() as u64;
    for (start, end, resource) in ranges {
        job.check()?;
        if start < cursor {
            return Err("NOTE_EXPORT_RESOURCE_INVALID：资源占位标识重叠。".into());
        }
        writer
            .write_all(&input.template.as_bytes()[cursor..start])
            .map_err(io)?;
        let (bytes, mime) = match resource {
            Resource::Font { name, .. } => {
                if !font_name_valid(name) {
                    return Err("NOTE_EXPORT_FONT_INVALID：字体名称无效。".into());
                }
                let asset = app
                    .asset_resolver()
                    .get(format!("resources/note-pdf-fonts/{name}"))
                    .ok_or("NOTE_EXPORT_FONT_UNAVAILABLE：随应用提供的字体缺失。")?;
                (asset.bytes, "font/ttf".to_owned())
            }
            Resource::Image { asset_id, .. } => {
                let asset = assets.read_export_asset(asset_id)?;
                let mime = asset.mime_type;
                if !mime.starts_with("image/")
                    || !mime
                        .bytes()
                        .all(|value| value.is_ascii_alphanumeric() || b"/-+.".contains(&value))
                {
                    return Err("NOTE_EXPORT_IMAGE_INVALID：图片资源 MIME 类型无效。".into());
                }
                (asset.bytes, mime)
            }
        };
        written = written.saturating_add(bytes.len() as u64 * 4 / 3 + 128);
        if written > MAX_EXPORT_BYTES {
            return Err("NOTE_EXPORT_TOO_LARGE：导出文件超过 256 MB。".into());
        }
        write!(writer, "data:{mime};base64,").map_err(io)?;
        let mut encoder = base64::write::EncoderWriter::new(
            &mut writer,
            &base64::engine::general_purpose::STANDARD,
        );
        for chunk in bytes.chunks(64 * 1024) {
            job.check()?;
            encoder.write_all(chunk).map_err(io)?;
        }
        encoder.finish().map_err(io)?;
        cursor = end;
    }
    job.check()?;
    writer
        .write_all(&input.template.as_bytes()[cursor..])
        .map_err(io)?;
    writer.flush().map_err(io)?;
    writer.get_ref().sync_all().map_err(io)?;
    Ok(())
}

/// The renderer only writes privately. Publish a complete sibling staging file
/// after cancellation loses the single commit boundary; never truncate a user file.
pub fn publish(
    source: &Path,
    destination: &Path,
    format: &str,
    job: &ExportJob,
) -> Result<(), String> {
    let mut input = File::open(source).map_err(io)?;
    if input.metadata().map_err(io)?.len() > MAX_EXPORT_BYTES {
        return Err("NOTE_EXPORT_TOO_LARGE：导出文件超过 256 MB。".into());
    }
    let mut header = [0_u8; 15];
    let length = input.read(&mut header).map_err(io)?;
    let expected = if format == "pdf" {
        b"%PDF-".as_slice()
    } else {
        b"<!doctype html>".as_slice()
    };
    if !header[..length].starts_with(expected) {
        return Err("NOTE_EXPORT_OUTPUT_INVALID：生成的文件格式无效。".into());
    }
    let mut temporary = tempfile::NamedTempFile::new_in(
        destination
            .parent()
            .ok_or("NOTE_EXPORT_DESTINATION_INVALID：保存目录无效。")?,
    )
    .map_err(io)?;
    temporary.write_all(&header[..length]).map_err(io)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        job.check()?;
        let count = input.read(&mut buffer).map_err(io)?;
        if count == 0 {
            break;
        }
        temporary.write_all(&buffer[..count]).map_err(io)?;
    }
    temporary.as_file().sync_all().map_err(io)?;
    job.begin_publish()?;
    temporary
        .persist(destination)
        .map_err(|error| io(error.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note_export::jobs::NoteExportJobs;
    fn job() -> std::sync::Arc<ExportJob> {
        let jobs = NoteExportJobs::default();
        let id = uuid::Uuid::new_v4().to_string();
        jobs.begin(&id).unwrap();
        jobs.get(&id).unwrap()
    }
    #[test]
    fn stopped_or_invalid_exports_keep_the_old_destination_and_remove_staging_files() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("private.pdf");
        let destination = root.path().join("chosen.pdf");
        std::fs::write(&source, b"%PDF-1.4 complete").unwrap();
        std::fs::write(&destination, b"existing file").unwrap();
        let stopped = job();
        assert!(stopped.cancel());
        assert!(
            publish(&source, &destination, "pdf", &stopped)
                .unwrap_err()
                .contains("NOTE_EXPORT_CANCELLED")
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing file");
        std::fs::write(&source, b"invalid").unwrap();
        assert!(publish(&source, &destination, "pdf", &job()).is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing file");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[test]
    fn complete_output_replaces_only_the_selected_file() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("private.html");
        let destination = root.path().join("chosen.html");
        std::fs::write(&source, b"<!doctype html><p>new</p>").unwrap();
        std::fs::write(&destination, b"old").unwrap();
        publish(&source, &destination, "html", &job()).unwrap();
        assert_eq!(
            std::fs::read(&destination).unwrap(),
            std::fs::read(&source).unwrap()
        );
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    }
    #[cfg(unix)]
    #[test]
    fn replacement_does_not_follow_a_destination_symlink() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("private.pdf");
        let destination = root.path().join("chosen.pdf");
        let other = root.path().join("other.pdf");
        std::fs::write(&source, b"%PDF-1.4 complete").unwrap();
        std::fs::write(&other, b"untouched").unwrap();
        std::os::unix::fs::symlink(&other, &destination).unwrap();
        publish(&source, &destination, "pdf", &job()).unwrap();
        assert_eq!(std::fs::read(&other).unwrap(), b"untouched");
        assert!(
            !std::fs::symlink_metadata(&destination)
                .unwrap()
                .is_symlink()
        );
    }
}
