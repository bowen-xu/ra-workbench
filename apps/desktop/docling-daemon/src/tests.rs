use super::*;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use research_docling_runtime::{ENGINE_COMMIT, ENGINE_VERSION, ModelFile, REQUIRED_FILES};
use tower::ServiceExt;
fn state() -> Arc<State> {
    let files = REQUIRED_FILES
        .iter()
        .copied()
        .chain(["licenses/NOTICE"])
        .map(|path| ModelFile {
            path: path.into(),
            sha256: "a".repeat(64),
            size_bytes: 1,
        })
        .collect();
    Arc::new(State {
        token: "fixture-token".into(),
        instance: "fixture-instance".into(),
        model: ModelManifest {
            schema_version: 1,
            engine_version: ENGINE_VERSION.into(),
            engine_commit: ENGINE_COMMIT.into(),
            precision: "fp32".into(),
            version: "fixture".into(),
            files,
        },
        lifecycle: Mutex::new(Lifecycle {
            leases: HashMap::new(),
            requests: 0,
            idle: None,
            draining: false,
        }),
        pipeline: Mutex::new(docling_pdf::Pipeline::new().unwrap()),
        tasks: conversion_tasks::Tasks::default(),
        inference: Arc::new(tokio::sync::Semaphore::new(1)),
        shutdown: Notify::new(),
        control: control::Control::default(),
    })
}

#[tokio::test]
async fn manual_shutdown_authenticates_cancels_all_owners_and_rejects_late_admission() {
    let state = state();
    let first = state.tasks.register("first", "client-a").unwrap();
    let second = state.tasks.register("second", "client-b").unwrap();
    state.lifecycle.lock().unwrap().requests = 2;
    let app = service::router(state.clone());
    let request = |auth: bool| {
        let mut request = Request::builder()
            .method("POST")
            .uri("/local-service/v1/shutdown");
        if auth {
            request = request.header("X-Api-Key", "fixture-token");
        }
        request.body(Body::empty()).unwrap()
    };
    assert_eq!(
        app.clone().oneshot(request(false)).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    assert!(!state.control.stopping());
    let response = app.clone().oneshot(request(true)).await.unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["operationId"], state.control.operation_id().unwrap());
    assert!(state.lifecycle.lock().unwrap().draining);
    assert!(first.cancel.load(std::sync::atomic::Ordering::Acquire));
    assert!(second.cancel.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(state.snapshot().unwrap()["state"], "draining");
    let repeated = app.clone().oneshot(request(true)).await.unwrap();
    let bytes = axum::body::to_bytes(repeated.into_body(), 8192)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["operationId"],
        body["operationId"]
    );
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/convert/source")
                .header("X-Api-Key", "fixture-token")
                .header("Content-Type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn model_cleanup_shutdown_requires_authentication_and_no_clients_or_requests() {
    let state = state();
    let app = service::router(state.clone());
    let request = |authorized: bool| {
        let mut request = Request::builder()
            .method("POST")
            .uri("/local-service/v1/shutdown-idle");
        if authorized {
            request = request.header("X-Api-Key", "fixture-token");
        }
        request.body(Body::empty()).unwrap()
    };
    assert_eq!(
        app.clone().oneshot(request(false)).await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    state.lifecycle.lock().unwrap().requests = 1;
    assert_eq!(
        app.clone().oneshot(request(true)).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    state.lifecycle.lock().unwrap().requests = 0;
    state.lifecycle.lock().unwrap().leases.insert(
        "other".into(),
        Lease {
            client: "other".into(),
            seen: Instant::now(),
        },
    );
    assert_eq!(
        app.clone().oneshot(request(true)).await.unwrap().status(),
        StatusCode::CONFLICT
    );
    assert!(!state.lifecycle.lock().unwrap().draining);
    state.lifecycle.lock().unwrap().leases.clear();
    assert_eq!(
        app.oneshot(request(true)).await.unwrap().status(),
        StatusCode::OK
    );
    assert!(state.lifecycle.lock().unwrap().draining);
}
#[tokio::test]
async fn status_and_conversion_require_authentication_and_live_lease() {
    let state = state();
    let app = service::router(state.clone());
    let reply = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/local-service/v1/status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::UNAUTHORIZED);
    let reply = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/local-service/v1/status")
                .header("X-Api-Key", "fixture-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::OK);
    let reply = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/convert/source")
                .header("Content-Type", "application/json")
                .header("X-Api-Key", "fixture-token")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(reply.status(), StatusCode::CONFLICT);
    assert_eq!(state.lifecycle.lock().unwrap().requests, 0);
}
#[test]
fn heartbeat_expiry_removes_only_stale_leases_and_keeps_active_work() {
    let state = state();
    {
        let mut life = state.lifecycle.lock().unwrap();
        life.requests = 1;
        life.leases.insert(
            "old".into(),
            Lease {
                client: "old-client".into(),
                seen: Instant::now() - Duration::from_secs(31),
            },
        );
        life.leases.insert(
            "live".into(),
            Lease {
                client: "live-client".into(),
                seen: Instant::now(),
            },
        );
    }
    let snapshot = state.snapshot().unwrap();
    assert_eq!(snapshot["activeClientCount"], 1);
    assert_eq!(snapshot["activeRequestCount"], 1);
}

#[tokio::test]
async fn queued_conversion_cancels_without_waiting_for_the_inference_slot() {
    let state = state();
    state.lifecycle.lock().unwrap().leases.insert(
        "lease-a".into(),
        Lease {
            client: "fixture".into(),
            seen: Instant::now(),
        },
    );
    let _busy = state.inference.clone().acquire_owned().await.unwrap();
    let app = service::router(state.clone());
    let id = Uuid::new_v4().to_string();
    let request = Request::builder()
        .method("POST")
        .uri("/v1/convert/source")
        .header("X-Api-Key", "fixture-token")
        .header("X-Docling-Lease-Id", "lease-a")
        .header("X-Docling-Request-Id", &id)
        .header("Content-Type", "application/json")
        .body(Body::from(
            json!({"sources":[{"kind":"file","base64_string":"JVBERi1maXh0dXJl"}]}).to_string(),
        ))
        .unwrap();
    let conversion = tokio::spawn(app.clone().oneshot(request));
    while state.lifecycle.lock().unwrap().requests == 0 {
        tokio::task::yield_now().await;
    }
    let cancel = Request::builder()
        .method("DELETE")
        .uri(format!("/local-service/v1/conversions/{id}"))
        .header("X-Api-Key", "fixture-token")
        .header("X-Docling-Lease-Id", "lease-a")
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(cancel).await.unwrap().status(), StatusCode::OK);
    let response = tokio::time::timeout(Duration::from_secs(1), conversion)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(response.status(), StatusCode::GONE);
    assert_eq!(state.lifecycle.lock().unwrap().requests, 0);
}

#[tokio::test]
async fn conversion_control_requires_authentication_and_the_owning_live_lease() {
    let state = state();
    for id in ["lease-a", "lease-b"] {
        state.lifecycle.lock().unwrap().leases.insert(
            id.into(),
            Lease {
                client: id.into(),
                seen: Instant::now(),
            },
        );
    }
    let id = Uuid::new_v4().to_string();
    let task = state.tasks.register(&id, "lease-a").unwrap();
    let app = service::router(state);
    let request = |token: &str, lease: &str| {
        Request::builder()
            .method("DELETE")
            .uri(format!("/local-service/v1/conversions/{id}"))
            .header("X-Api-Key", token)
            .header("X-Docling-Lease-Id", lease)
            .body(Body::empty())
            .unwrap()
    };
    assert_eq!(
        app.clone()
            .oneshot(request("wrong", "lease-a"))
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.clone()
            .oneshot(request("fixture-token", "lease-b"))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    assert!(!task.cancel.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(
        app.oneshot(request("fixture-token", "lease-a"))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(task.cancel.load(std::sync::atomic::Ordering::Acquire));
}
