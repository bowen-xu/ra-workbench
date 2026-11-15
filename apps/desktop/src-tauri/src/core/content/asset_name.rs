/// New image references use the storage identity; attachment names stay with their owner.
pub(crate) fn note_image_display_name(asset_id: &str, mime_type: &str) -> Option<String> {
    let mime_type = mime_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if !mime_type.starts_with("image/") {
        return None;
    }
    let extension = match mime_type.as_str() {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        "image/avif" => "avif",
        "image/apng" => "apng",
        "image/bmp" | "image/x-ms-bmp" => "bmp",
        "image/tiff" => "tiff",
        "image/x-icon" | "image/vnd.microsoft.icon" => "ico",
        "image/heic" => "heic",
        "image/heif" => "heif",
        "image/jxl" => "jxl",
        _ => "bin",
    };
    Some(format!("{asset_id}.{extension}"))
}

#[cfg(test)]
mod tests {
    use super::note_image_display_name;

    #[test]
    fn image_names_follow_mime_instead_of_a_source_filename() {
        for (mime_type, extension) in [
            ("image/png", "png"),
            ("image/jpeg", "jpg"),
            ("image/gif", "gif"),
            ("image/webp", "webp"),
            ("image/svg+xml", "svg"),
            ("image/avif", "avif"),
            ("image/apng", "apng"),
            ("image/bmp", "bmp"),
            ("image/tiff", "tiff"),
            ("image/x-icon", "ico"),
            ("image/vnd.microsoft.icon", "ico"),
            ("image/heic", "heic"),
            ("image/heif", "heif"),
            ("image/jxl", "jxl"),
            ("IMAGE/JPEG; charset=binary", "jpg"),
            ("image/future-format", "bin"),
        ] {
            assert_eq!(
                note_image_display_name("asset-id", mime_type),
                Some(format!("asset-id.{extension}"))
            );
        }
        for mime_type in [
            "text/plain",
            "application/pdf",
            "application/octet-stream",
            "",
        ] {
            assert_eq!(note_image_display_name("asset-id", mime_type), None);
        }
    }
}
