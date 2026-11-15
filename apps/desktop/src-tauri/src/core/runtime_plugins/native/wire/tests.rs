use super::*;
use research_workbench_native_sdk::retrieval_trace::{MarkSentCondition, TraceRejection};

fn writing_request() -> writing::Request {
    writing::Request {
        protocol_version: 1,
        plugin_id: "writing_projects".into(),
        package_key: "a".repeat(64),
        instance_id: "8cbfd428-3ac1-4bce-b4f6-d41f31063d8e".into(),
        request_id: "590c18c0-52f2-4905-8e39-23e082778384".into(),
        body: writing::RequestBody::Initialize,
    }
}
fn literature_request() -> literature::Request {
    let mut request = literature::Request {
        protocol_version: 2,
        plugin_id: "literature_library".into(),
        package_key: "a".repeat(64),
        instance_id: "8cbfd428-3ac1-4bce-b4f6-d41f31063d8e".into(),
        request_id: "590c18c0-52f2-4905-8e39-23e082778384".into(),
        body: literature::RequestBody::Initialize,
    };
    request.body = literature::RequestBody::DecideNoteTrace {
        snapshot: literature::SnapshotRef::for_request(&request, 30, "b".repeat(64)),
    };
    request
}
fn frame(bytes: &[u8]) -> &[u8] {
    &bytes[..bytes.len() - 1]
}

#[test]
fn finite_adapter_keeps_both_sdk_codecs_and_expected_response_kinds() {
    let writing = writing_request();
    let wire = WireRequest::Writing(writing.clone());
    assert_eq!(
        wire.encode().unwrap().as_slice(),
        writing::encode_request(&writing).unwrap()
    );
    let response = writing::Response::for_request(
        &writing,
        writing::ResponseBody::Initialized {
            capability: writing::CAPABILITY_ID.into(),
            version: 1,
        },
    );
    assert!(matches!(
        wire.decode_response(frame(&writing::encode_response(&response).unwrap()))
            .unwrap(),
        WireResponse::Writing(writing::ResponseBody::Initialized { .. })
    ));
    let literature = literature_request();
    let wire = WireRequest::Literature(literature.clone());
    assert_eq!(
        wire.encode().unwrap(),
        literature::encode_request(&literature).unwrap()
    );
    let response = literature::Response::for_request(
        &literature,
        literature::ResponseBody::NoteTraceDecision {
            decision: literature::TraceDecision::Ready {
                mark_sent: MarkSentCondition::ORIGINAL,
            },
        },
    );
    assert!(matches!(
        wire.decode_response(frame(&literature::encode_response(&response).unwrap()))
            .unwrap(),
        WireResponse::Literature(literature::ResponseBody::NoteTraceDecision { .. })
    ));
}

#[test]
fn finite_adapter_rejects_identity_kind_cross_profile_and_malformed_ready() {
    let request = literature_request();
    let wire = WireRequest::Literature(request.clone());
    let response = literature::Response::for_request(
        &request,
        literature::ResponseBody::NoteTraceDecision {
            decision: literature::TraceDecision::Rejected {
                reason: TraceRejection::Expired,
            },
        },
    );
    for field in ["packageKey", "instanceId", "requestId"] {
        let mut value = serde_json::to_value(&response).unwrap();
        value[field] = if field == "packageKey" {
            serde_json::json!("b".repeat(64))
        } else {
            serde_json::json!("bc27daa2-3ef5-43b5-bc5b-e9ca02081b12")
        };
        assert_eq!(
            wire.decode_response(&serde_json::to_vec(&value).unwrap())
                .err()
                .unwrap()
                .code,
            "plugin_host_response_mismatch"
        );
    }
    let response =
        literature::Response::for_request(&request, literature::ResponseBody::ShutdownComplete);
    assert_eq!(
        wire.decode_response(frame(&literature::encode_response(&response).unwrap()))
            .err()
            .unwrap()
            .code,
        "plugin_host_response_mismatch"
    );
    let response = writing::Response::for_request(
        &writing_request(),
        writing::ResponseBody::Initialized {
            capability: writing::CAPABILITY_ID.into(),
            version: 1,
        },
    );
    assert_eq!(
        wire.decode_response(frame(&writing::encode_response(&response).unwrap()))
            .err()
            .unwrap()
            .code,
        "plugin_host_protocol_invalid"
    );
    for condition in [
        MarkSentCondition {
            expected_rows: 0,
            ..MarkSentCondition::ORIGINAL
        },
        MarkSentCondition {
            on_mismatch: TraceRejection::Expired,
            ..MarkSentCondition::ORIGINAL
        },
    ] {
        let response = literature::Response::for_request(
            &request,
            literature::ResponseBody::NoteTraceDecision {
                decision: literature::TraceDecision::Ready {
                    mark_sent: condition,
                },
            },
        );
        let bytes = serde_json::to_vec(&response).unwrap();
        assert_eq!(
            wire.decode_response(&bytes).err().unwrap().code,
            "plugin_host_protocol_invalid"
        );
    }
}
