//! Owned HTTP data at the existing admitted blocking boundary.
use hyper::body::Bytes;
use research_workbench_native_sdk::{browser_capture::*, catalog_save::PreparedDoi};
use serde_json::Value;

pub(crate) struct CaptureRequest {
    pub(crate) route: String,
    pub(crate) payload: Value,
    pub(crate) session_id: Option<String>,
    pub(crate) metadata: Option<Value>,
    pub(crate) bytes: Bytes,
}
impl CaptureRequest {
    pub(crate) fn into_input(self, scope: &str) -> (BrowserCaptureInputV1, Bytes) {
        use BrowserCaptureInputV1 as Input;
        let scope = scope.to_owned();
        let input = match self.route.as_str() {
            "saveItems" => {
                let doi = self
                    .payload
                    .get("items")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|raw| {
                        let raw = raw.get("DOI").and_then(Value::as_str).unwrap_or_default();
                        let display =
                            crate::doi::clean_doi(raw).unwrap_or_else(|| raw.trim().to_owned());
                        PreparedDoi {
                            normalized: crate::doi::normalized_doi(&display),
                            display,
                        }
                    })
                    .collect();
                Input::Items {
                    scope,
                    payload: self.payload,
                    doi,
                }
            }
            "saveSnapshot" => Input::Snapshot {
                scope,
                payload: self.payload,
            },
            "saveAttachment" | "saveStandaloneAttachment" => {
                use sha2::{Digest, Sha256};
                let pdf = PdfObservationV1 {
                    byte_size: self.bytes.len(),
                    starts_with_pdf: self.bytes.starts_with(b"%PDF-"),
                    sha256: format!("{:x}", Sha256::digest(&self.bytes)),
                };
                let session_id = self.session_id.unwrap_or_default();
                let metadata = self.metadata.unwrap_or(Value::Null);
                if self.route == "saveAttachment" {
                    Input::Attachment {
                        scope,
                        session_id,
                        metadata,
                        pdf,
                    }
                } else {
                    Input::Standalone {
                        scope,
                        session_id,
                        metadata,
                        pdf,
                    }
                }
            }
            "sessionStatus" => Input::SessionStatus {
                scope,
                session_id: self
                    .payload
                    .get("sessionID")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            },
            "updateSession" => Input::UpdateSession {
                scope,
                payload: self.payload,
            },
            "getRecognizedItem" => Input::RecognizedItem {
                scope,
                payload: self.payload,
            },
            "getSelectedCollection" => Input::SelectedLibrary {},
            "getClientHostnames" | "proxies" => Input::ClientHostnames {},
            "delaySync" => Input::DelaySync {},
            "hasAttachmentResolvers" => Input::AttachmentResolvers {},
            _ => Input::Unsupported {},
        };
        (input, self.bytes)
    }
}
