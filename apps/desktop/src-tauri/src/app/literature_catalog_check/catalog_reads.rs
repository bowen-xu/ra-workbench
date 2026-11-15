//! The original coherent catalog acceptance case, moved without body changes.
use super::*;

pub(super) async fn check(
    store: &AppStore,
    service: &LiteratureTraceService,
    key: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let trace_input = || SnapshotInput {
        query: "synthetic absent trace".into(),
        row: None,
    };
    let initial = service.decide(key.into(), trace_input()).await?;
    if initial.decision
        != (TraceDecision::Rejected {
            reason: TraceRejection::NotFound,
        })
    {
        return Err("original trace decision changed before catalog reads".into());
    }
    let list = |query: Option<&str>| CatalogRequest::List {
        query: query.map(str::to_owned),
    };
    let read = |request| service.catalog(store.shared_agent_connection(), key.into(), request);
    let all = read(list(None)).await?;
    if ids(&all) != ["paper-new", "paper-old"] || read(list(Some(" \t "))).await? != all {
        return Err("catalog list order or empty-query semantics changed".into());
    }
    for (query, expected) in [
        ("needle", vec!["paper-new"]),
        ("2025", vec!["paper-new"]),
        ("absent", vec![]),
    ] {
        let actual = read(list(Some(query))).await?;
        if ids(&actual) != expected {
            return Err(format!(
                "synthetic catalog query {query:?}: expected {expected:?}, received {:?}",
                ids(&actual)
            )
            .into());
        }
    }
    let one = read(CatalogRequest::Get {
        id: "paper-new".into(),
    })
    .await?;
    if one != all[..1] || serde_json::to_value(&one[0])? != fixture::expected() {
        return Err(
            "complete catalog DTO differs from the original fields/order/null rules".into(),
        );
    }
    for id in ["missing", "paper-trashed", "paper-archived"] {
        let error = read(CatalogRequest::Get { id: id.into() })
            .await
            .unwrap_err();
        if error.code != "not_found"
            || error.package_key.as_deref() != Some(key)
            || error.instance_id.is_none()
            || error.request_id.is_none()
        {
            return Err("catalog absence diagnostic lost its identity".into());
        }
    }
    // A result larger than the pipe-frame budget must retain every text byte.
    let large = "x".repeat(64 * 1024 * 1024 + 1);
    store.connection()?.execute(
        "update bibliographic_items set abstract_text=?1 where id='paper-new'",
        [&large],
    )?;
    let output = read(CatalogRequest::Get {
        id: "paper-new".into(),
    })
    .await?;
    if output[0].abstract_text != large {
        return Err("large catalog text was truncated".into());
    }
    let final_trace = service.decide(key.into(), trace_input()).await?;
    if final_trace.decision != initial.decision
        || final_trace.instance_id != initial.instance_id
        || final_trace.request_id == initial.request_id
    {
        return Err("catalog reads replaced or invalidated the original trace host".into());
    }
    Ok(
        json!({"listOrder":ids(&all),"serializedFields":fixture::expected().as_object().map(|fields| fields.len()),"searchCases":3,
        "missingCases":3,"largeTextBytes":large.len(),
        "largeTextSha256":format!("{:x}",Sha256::digest(large.as_bytes())),
        "sameHostTraceProfile":true,"instanceId":initial.instance_id,
        "traceBeforeRequestId":initial.request_id,"traceAfterRequestId":final_trace.request_id,
        "coreOnly":true}),
    )
}
