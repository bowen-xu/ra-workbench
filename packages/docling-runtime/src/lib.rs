//! Versioned, platform-independent model resource contract shared with the native worker.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs::File, io::Read, path::Path};

pub const ENGINE_VERSION: &str = "2.45.7";
pub const ENGINE_COMMIT: &str = "a90192cd3096b5db26db928ded4d3f024d7e4ed4";
/// Application assembly semantics; changing this must fence services and parse caches.
pub const PARSER_REVISION: u32 = 2;
/// Service control capabilities; fences daemons without invalidating parse caches or model bundles.
pub const SERVICE_REVISION: u32 = 2;
pub const PROTOCOL_VERSION: u32 = 2;
pub const DEFAULT_MANIFEST_URL: &str = "http://127.0.0.1:4174/models/docling/latest.json";
pub const REQUIRED_FILES: &[&str] = &[
    "layout_heron.onnx",
    "ocr_det.onnx",
    "ocr_rec_v6.onnx",
    "ocr_rec_v6_dict.txt",
    "ocr_textline_ori.onnx",
    "tableformer/encoder.onnx",
    "tableformer/decoder.onnx",
    "tableformer/bbox.onnx",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelFile {
    pub path: String,
    pub sha256: String,
    pub size_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelManifest {
    pub schema_version: u32,
    pub engine_version: String,
    pub engine_commit: String,
    pub precision: String,
    pub version: String,
    pub files: Vec<ModelFile>,
}

impl ModelManifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1
            || self.engine_version != ENGINE_VERSION
            || self.engine_commit != ENGINE_COMMIT
            || self.precision != "fp32"
        {
            return Err("模型包与当前 Rust Docling FP32 引擎不兼容".into());
        }
        if self.version.is_empty()
            || self.version.len() > 100
            || !self.version.as_bytes()[0].is_ascii_alphanumeric()
            || !self
                .version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        {
            return Err("模型版本无效".into());
        }
        let mut paths = HashSet::new();
        let mut total = 0u64;
        for file in &self.files {
            let allowed = REQUIRED_FILES.contains(&file.path.as_str())
                || [
                    "tableformer/encoder.onnx.data",
                    "tableformer/decoder.onnx.data",
                    "tableformer/bbox.onnx.data",
                ]
                .contains(&file.path.as_str())
                || (file.path.starts_with("licenses/")
                    && !matches!(&file.path[9..], "." | "..")
                    && file.path[9..]
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                    && file.path.len() > 9);
            if !allowed
                || !paths.insert(file.path.as_str())
                || file.size_bytes == 0
                || file.size_bytes > 2 * 1024 * 1024 * 1024
                || file.sha256.len() != 64
                || !file
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
                return Err("模型文件路径、大小或校验和无效".into());
            }
            total = total.checked_add(file.size_bytes).ok_or("模型包过大")?;
        }
        if total > 4 * 1024 * 1024 * 1024
            || self.files.len() > 32
            || REQUIRED_FILES.iter().any(|path| !paths.contains(path))
            || !paths.iter().any(|path| path.starts_with("licenses/"))
        {
            return Err("模型包不完整或过大（需要全部 FP32 模型及许可）".into());
        }
        Ok(())
    }

    /// Content identity is independent of an operator's display version.
    pub fn identity(&self) -> Result<String, String> {
        self.validate()?;
        let mut files = self.files.clone();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let bytes = serde_json::to_vec(&(ENGINE_VERSION, ENGINE_COMMIT, "fp32", files))
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "docling-rs:{ENGINE_VERSION}:fp32:{:x}",
            Sha256::digest(bytes)
        ))
    }

    pub fn directory_name(&self) -> Result<String, String> {
        Ok(self
            .identity()?
            .rsplit(':')
            .next()
            .ok_or("模型身份无效")?
            .into())
    }

    /// Same model files, different parser semantics. Does not alter model paths
    /// or require republishing/downloading the model bundle.
    pub fn execution_identity(&self) -> Result<String, String> {
        Ok(format!("{}:parser={PARSER_REVISION}", self.identity()?))
    }

    pub fn instance_directory_name(&self) -> Result<String, String> {
        Ok(format!(
            "{}-parser-{PARSER_REVISION}-service-{SERVICE_REVISION}",
            self.directory_name()?
        ))
    }

    pub fn verify(&self, root: &Path) -> Result<(), String> {
        self.validate()?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        for file in &self.files {
            let path = root
                .join(&file.path)
                .canonicalize()
                .map_err(|_| format!("模型文件缺失：{}", file.path))?;
            if !path.starts_with(&root) {
                return Err("模型文件越界".into());
            }
            let (size, hash) = hash_file(&path)?;
            if size != file.size_bytes || hash != file.sha256 {
                return Err(format!("模型文件校验失败：{}", file.path));
            }
        }
        Ok(())
    }
}

pub fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let mut input = File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut size = 0;
    let mut bytes = [0u8; 64 * 1024];
    loop {
        let count = input.read(&mut bytes).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&bytes[..count]);
        size += count as u64;
    }
    Ok((size, format!("{:x}", digest.finalize())))
}
