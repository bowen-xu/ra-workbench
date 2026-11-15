use tauri::AppHandle;

fn validate_asset_name(name: &str) -> Result<(), String> {
    let is_spelling = name.len() == 16
        && name.starts_with("spelling-")
        && name.ends_with(".json")
        && name.as_bytes()[9..11].iter().all(u8::is_ascii_digit)
        && name[9..11]
            .parse::<u8>()
            .is_ok_and(|length| (3..=32).contains(&length));
    let is_shard = name.len() == 7
        && name.ends_with(".json")
        && name.as_bytes()[..2]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
    if name == "manifest.json" || is_shard || is_spelling {
        Ok(())
    } else {
        Err("无效的词库资源名称".into())
    }
}

/// Resolves only immutable dictionary assets, never arbitrary application/user paths.
#[tauri::command]
pub async fn read_dictionary_asset(app: AppHandle, name: String) -> Result<String, String> {
    validate_asset_name(&name)?;
    tauri::async_runtime::spawn_blocking(move || {
        let asset = app
            .asset_resolver()
            .get(format!("dictionary/ecdict-v2/{name}"))
            .ok_or_else(|| "本地词库资源缺失，请重新准备词库".to_string())?;
        if asset.bytes.len() > 2_000_000 {
            return Err("本地词库资源大小无效".into());
        }
        String::from_utf8(asset.bytes).map_err(|_| "本地词库编码无效".to_string())
    })
    .await
    .map_err(|_| "无法读取本地词库".to_string())?
}

#[cfg(test)]
mod tests {
    use super::validate_asset_name;

    #[test]
    fn accepts_only_manifest_and_fixed_hex_shards() {
        for name in [
            "manifest.json",
            "00.json",
            "af.json",
            "ff.json",
            "spelling-03.json",
            "spelling-32.json",
        ] {
            assert!(validate_asset_name(name).is_ok());
        }
        for name in [
            "../manifest.json",
            "../../confidential/key",
            "/00.json",
            "ff.json/..",
            "ff.JSON",
            "gg.json",
            "f.json",
            "fff.json",
            "LICENSE.txt",
            "index.html",
            "file:///ff.json",
            "https://example.com/ff.json",
            "ff.json\0",
            "spelling-02.json",
            "spelling-33.json",
            "spelling-3.json",
            "spelling-+3.json",
            "\u{00e9}.json",
        ] {
            assert!(validate_asset_name(name).is_err(), "accepted {name:?}");
        }
    }
}
