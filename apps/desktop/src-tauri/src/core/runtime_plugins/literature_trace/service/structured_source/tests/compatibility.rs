use super::*;

async fn original_tool_read(fixture: &Fixture, call: ToolCall) -> Value {
    let arguments = call.arguments.clone();
    let frozen = crate::agent::source_tools::OwnedSubmissionTools::new(fixture.record.clone());
    let (_, executor) = crate::agent::context_session::resource_tools::registration(
        fixture.store.clone(),
        fixture.record.clone(),
        OwnedToolRegistration::new(frozen.definitions(), Arc::new(frozen)),
    )
    .into_parts();
    let output = executor.authorize_and_execute(call).await.unwrap();
    assert!(
        !output.is_error,
        "runtime first read failed for {arguments}. {}",
        output.content
    );
    let mut value: Value = serde_json::from_str(&output.content).unwrap();
    // Verify the admitted turn before checking source text and character ranges.
    assert_eq!(value["turnId"], fixture.record.turn_id.to_string());
    value.as_object_mut().unwrap().remove("turnId");
    value
}

async fn read_first(arguments: Value) -> (Value, f64, String) {
    let mut runtime = Fixture::new();
    let document = json!({
        "pages":{"1":{"page_no":1,"size":{"width":612,"height":792}}},
        "texts":[{"self_ref":"#/texts/0","text":"Original source 证据 α",
            "prov":[{"page_no":1,"bbox":{"l":1.23318e-7,"t":10,"r":500,"b":50},"charspan":[0,9]}]}]
    })
    .to_string();
    runtime.raw_parse(&document);
    let prepared = runtime.prepared();
    let expected_id = prepared.projection.snapshot_id.clone();
    let expected_hash = prepared.projection.context_hash.clone();
    let raw_x = prepared.projection.items[0].regions[0].rect.x;
    runtime.configure(&[prepared.build, prepared.project], None, false);
    assert!(runtime.store.require_paper_reading_source().is_err());
    let mut call = runtime.call.clone();
    call.arguments = arguments;
    let actual = original_tool_read(&runtime, call).await;
    {
        let db = runtime.store.connection().unwrap();
        let stored: (String, String) = db
            .query_row(
                "select id,context_hash from paper_context_snapshots",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(stored, (expected_id.clone(), expected_hash.clone()));
    }
    runtime.service.shutdown().await.unwrap();
    assert_eq!(runtime.counts(), (1, 1));
    assert_eq!(
        runtime
            .lifecycle()
            .lines()
            .filter(|line| line.starts_with("call "))
            .count(),
        2
    );
    (actual, raw_x, expected_id)
}

#[tokio::test]
async fn first_freeze_and_paged_tool_text_match_original_json_readback_at_small_coordinate_threshold()
 {
    let (full, raw_x, expected_id) = read_first(json!({
        "fileId":"selected", "view":"structured", "maxChars":8000,
    }))
    .await;
    let text = full["text"].as_str().unwrap();
    let original_body: Value = serde_json::from_str(text).unwrap();
    let returned_x = original_body["items"][0]["regions"][0]["rect"]["x"]
        .as_f64()
        .unwrap();
    assert_ne!(
        raw_x.to_bits(),
        returned_x.to_bits(),
        "fixture must exercise the original JSON float readback boundary"
    );
    assert_eq!(original_body["snapshotId"], expected_id);
    let coordinate_byte = text.find("\"x\"").unwrap();
    let start = text[..coordinate_byte].chars().count();
    // Exercise pagination with a fresh admitted turn so the initial publication
    // retains the same supported inputs and original protection boundary.
    let (page, page_raw_x, page_id) = read_first(json!({
        "fileId":"selected", "view":"structured", "startChar":start, "maxChars":17,
    }))
    .await;
    assert_eq!(page_id, expected_id);
    assert_eq!(page_raw_x.to_bits(), raw_x.to_bits());
    assert_eq!(page["startChar"], start);
    assert_eq!(page["endChar"], start + 17);
    assert_eq!(page["nextChar"], start + 17);
    assert_eq!(page["totalChars"], text.chars().count());
    assert_eq!(page["readViewId"], full["readViewId"]);
    assert_eq!(
        page["text"],
        text.chars().skip(start).take(17).collect::<String>()
    );
}
