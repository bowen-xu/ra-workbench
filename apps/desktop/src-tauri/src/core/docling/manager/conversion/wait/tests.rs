use super::*;
use crate::docling::error::DoclingCommandError;
use serde_json::json;
use std::future::{pending, ready};

fn progress() -> ParseProgress {
    ParseProgress {
        stage: "parsing".into(),
        completed_pages: 2,
        total_pages: 16,
    }
}

fn no_response() -> impl Future<Output = Result<Value, DoclingError>> {
    pending()
}

#[tokio::test(start_paused = true)]
async fn unchanged_page_responses_keep_a_long_conversion_alive() {
    let control = ParseControl::new();
    let started = Instant::now();
    let result = for_response(
        async {
            tokio::time::sleep(Duration::from_secs(35)).await;
            Ok(json!({"status":"success"}))
        },
        || ready(Ok(progress())),
        || ready(Ok(())),
        &control,
        10_000,
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "success");
    assert_eq!(started.elapsed(), Duration::from_secs(35));
    assert_eq!(control.snapshot().completed_pages, 2);
}

#[tokio::test(start_paused = true)]
async fn failed_polls_time_out_since_the_last_valid_response_with_diagnostics() {
    let control = ParseControl::new();
    let started = Instant::now();
    let mut calls = 0;
    let error = for_response(
        no_response(),
        || {
            calls += 1;
            ready(if calls <= 2 {
                Ok(progress())
            } else {
                Err(DoclingError::Request("Docling status HTTP 503".into()))
            })
        },
        || ready(Ok(())),
        &control,
        10_000,
    )
    .await
    .unwrap_err();
    assert_eq!(started.elapsed(), Duration::from_millis(10_300));
    let error = serde_json::to_value(DoclingCommandError::from(error)).unwrap();
    assert_eq!(error["code"], "response_timeout");
    let message = error["message"].as_str().unwrap();
    for expected in [
        "连续 10000 毫秒",
        "有效任务状态响应",
        &control.id,
        "HTTP 503",
    ] {
        assert!(message.contains(expected), "{message}");
    }
}

#[tokio::test(start_paused = true)]
async fn invalid_status_does_not_extend_the_deadline() {
    for invalid in [
        ParseProgress {
            stage: "unknown".into(),
            ..progress()
        },
        ParseProgress {
            completed_pages: 17,
            ..progress()
        },
    ] {
        let control = ParseControl::new();
        let started = Instant::now();
        let error = for_response(
            no_response(),
            || ready(Ok(invalid.clone())),
            || ready(Ok(())),
            &control,
            10_000,
        )
        .await
        .unwrap_err();
        assert_eq!(started.elapsed(), Duration::from_secs(10));
        assert!(matches!(error, DoclingError::ResponseTimeout { .. }));
    }
}

#[tokio::test(start_paused = true)]
async fn a_stuck_poll_cannot_block_the_deadline() {
    let started = Instant::now();
    let error = for_response(
        no_response(),
        pending,
        || ready(Ok(())),
        &ParseControl::new(),
        10_000,
    )
    .await
    .unwrap_err();
    assert_eq!(started.elapsed(), Duration::from_secs(10));
    assert!(matches!(error, DoclingError::ResponseTimeout { .. }));
}

#[tokio::test(start_paused = true)]
async fn a_stuck_poll_cannot_block_a_completed_conversion() {
    let result = for_response(
        async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(json!({"status":"success"}))
        },
        pending,
        || ready(Ok(())),
        &ParseControl::new(),
        10_000,
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "success");
}

#[tokio::test(start_paused = true)]
async fn cancellation_interrupts_polling_but_waits_for_conversion_confirmation() {
    let control = ParseControl::new();
    control.cancel();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let mut tx = Some(tx);
    let started = Instant::now();
    let error = for_response(
        async {
            rx.await.unwrap();
            tokio::time::sleep(Duration::from_secs(1)).await;
            Err(DoclingError::Cancelled)
        },
        pending,
        || {
            tx.take().expect("cancel sent once").send(()).unwrap();
            ready(Ok(()))
        },
        &control,
        10_000,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, DoclingError::Cancelled));
    assert_eq!(started.elapsed(), Duration::from_secs(1));
}
