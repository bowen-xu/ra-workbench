use hyper::{HeaderMap, StatusCode};
use serde_json::{Value, json};

pub(super) const TOKEN_HEADER: &str = "x-research-assistant-token";
pub(super) const INSTANCE_HEADER: &str = "x-research-assistant-instance-id";
pub(super) const API_VERSION: &str = "2";
pub(super) const MAX_PDF_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_JSON_BYTES: usize = 1024 * 1024;

pub(super) fn extension_origin(value: &str) -> bool {
    value
        .strip_prefix("chrome-extension://")
        .is_some_and(|id| id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte)))
}

pub(super) fn validate_headers(headers: &HeaderMap, host: &str) -> Result<(), StatusCode> {
    if headers.get_all("host").iter().count() != 1 || headers.get_all("origin").iter().count() > 1 {
        return Err(StatusCode::BAD_REQUEST);
    }
    if headers.get("host").and_then(|value| value.to_str().ok()) != Some(host) {
        return Err(StatusCode::BAD_REQUEST);
    }
    if let Some(origin) = headers.get("origin")
        && !origin.to_str().is_ok_and(extension_origin)
    {
        return Err(StatusCode::FORBIDDEN);
    }
    if headers
        .get_all("x-zotero-connector-api-version")
        .iter()
        .count()
        != 1
        || headers
            .get("x-zotero-connector-api-version")
            .and_then(|value| value.to_str().ok())
            != Some("3")
    {
        return Err(StatusCode::PRECONDITION_FAILED);
    }
    if headers
        .get_all("x-research-assistant-connector-api-version")
        .iter()
        .count()
        != 1
        || headers
            .get("x-research-assistant-connector-api-version")
            .and_then(|value| value.to_str().ok())
            != Some(API_VERSION)
    {
        return Err(StatusCode::PRECONDITION_FAILED);
    }
    Ok(())
}

pub(super) fn session_id(query: Option<&str>) -> Result<String, StatusCode> {
    let query = query.ok_or(StatusCode::BAD_REQUEST)?;
    let (key, value) = query.split_once('=').ok_or(StatusCode::BAD_REQUEST)?;
    if key != "sessionID" || value.contains('&') || value.is_empty() || value.len() > 768 {
        return Err(StatusCode::BAD_REQUEST);
    }
    percent_encoding::percent_decode_str(value)
        .decode_utf8()
        .map(|value| value.into_owned())
        .map_err(|_| StatusCode::BAD_REQUEST)
}

pub(super) fn attachment_metadata(headers: &HeaderMap) -> Result<Value, StatusCode> {
    let content_type = headers
        .get("content-type")
        .and_then(|value| value.to_str().ok());
    if content_type != Some("application/pdf") {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
    let raw = headers
        .get("x-metadata")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 16384)
        .ok_or(StatusCode::BAD_REQUEST)?;
    let mut metadata: Value = serde_json::from_str(raw).map_err(|_| StatusCode::BAD_REQUEST)?;
    let object = metadata.as_object_mut().ok_or(StatusCode::BAD_REQUEST)?;
    if object
        .get("contentType")
        .and_then(Value::as_str)
        .is_some_and(|value| value != "application/pdf")
    {
        return Err(StatusCode::UNSUPPORTED_MEDIA_TYPE);
    }
    if let Some(title) = object.get("title").and_then(Value::as_str) {
        let title = decode_title(title)?;
        object.insert("title".to_owned(), Value::String(title));
    }
    Ok(metadata)
}

// Upstream itemSaver_background uses a single RFC2047 UTF-8 Q word for titles.
// This is not a generic MIME-header decoder: reject incomplete/non-UTF8 words.
fn decode_title(value: &str) -> Result<String, StatusCode> {
    if value.len() > 8192 {
        return Err(StatusCode::BAD_REQUEST);
    }
    if !value.starts_with("=?") {
        return Ok(value.to_owned());
    }
    let encoded = value
        .strip_prefix("=?UTF-8?Q?")
        .and_then(|value| value.strip_suffix("?="))
        .ok_or(StatusCode::BAD_REQUEST)?;
    let mut result = Vec::with_capacity(encoded.len());
    let mut bytes = encoded.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'_' => result.push(b' '),
            b'=' => {
                let high = bytes.next().and_then(|byte| (byte as char).to_digit(16));
                let low = bytes.next().and_then(|byte| (byte as char).to_digit(16));
                result.push(
                    ((high.ok_or(StatusCode::BAD_REQUEST)? << 4)
                        | low.ok_or(StatusCode::BAD_REQUEST)?) as u8,
                );
            }
            byte => result.push(byte),
        }
    }
    String::from_utf8(result).map_err(|_| StatusCode::BAD_REQUEST)
}

pub(super) fn ping(shared: &super::Shared) -> Value {
    json!({"application":"research-assistant", "version":env!("CARGO_PKG_VERSION"),
        "connectorApiVersion":2, "zoteroConnectorApiVersion":3,
        "environment":shared.environment, "receiverId":shared.identity.id(),
        "instanceId":shared.instance_id, "connectionToken":shared.connection_token,
        "prefs": {"downloadAssociatedFiles":true,
        "automaticSnapshots":false, "reportActiveURL":false, "supportsTagsAutocomplete":false,
        "canUserAddNote":false}, "capabilities":{"pdf":true,"snapshots":false,
        "openAccessResolver":false,"documentRecognition":false}})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_origin_and_host_are_checked_independently_of_the_secret() {
        assert!(extension_origin(&format!(
            "chrome-extension://{}",
            "a".repeat(32)
        )));
        for value in [
            "null",
            "https://example.org",
            "chrome-extension://abc",
            "chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/",
        ] {
            assert!(!extension_origin(value));
        }
        let mut headers = HeaderMap::new();
        headers.insert("host", "127.0.0.1:23121".parse().expect("host"));
        headers.insert(
            "x-zotero-connector-api-version",
            "3".parse().expect("version"),
        );
        assert_eq!(
            validate_headers(&headers, "127.0.0.1:23121"),
            Err(StatusCode::PRECONDITION_FAILED)
        );
        headers.insert(
            "x-research-assistant-connector-api-version",
            "2".parse().expect("version"),
        );
        assert!(validate_headers(&headers, "127.0.0.1:23121").is_ok());
        assert_eq!(
            validate_headers(&headers, "127.0.0.1:23120"),
            Err(StatusCode::BAD_REQUEST)
        );
        headers.insert("origin", "https://example.org".parse().expect("origin"));
        assert_eq!(
            validate_headers(&headers, "127.0.0.1:23121"),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn attachment_title_encoding_and_session_parameter_have_strict_boundaries() {
        assert_eq!(
            decode_title("=?UTF-8?Q?=E8=AE=BA=E6=96=87_PDF?=").expect("decode"),
            "论文 PDF"
        );
        assert!(decode_title("=?UTF-8?Q?=GG?=").is_err());
        assert!(decode_title("=?UTF-8?Q?=FF?=").is_err());
        assert_eq!(
            session_id(Some("sessionID=one%20two")).expect("id"),
            "one two"
        );
        for value in [
            "sessionID=a&sessionID=b",
            "url=https://example.org",
            "sessionID=",
        ] {
            assert!(session_id(Some(value)).is_err());
        }
    }
}
