use super::*;

fn frozen_tools(store: &AppStore, conversation: Uuid) -> (ResourceTools, String, String) {
    let (target, denied) = {
        let db = store.connection().unwrap();
        let sha = "a".repeat(64);
        db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values(?1,'frozen.pdf','application/pdf',1,'now')",[&sha]).unwrap();
        db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values('frozen-pdf','paper',?1,'Frozen.pdf','now')",[&sha]).unwrap();
        db.execute("insert into ai_runs(id,task_kind,status,provider_id,model_id,pipeline_version,prompt_version,output_schema_version,scope_hash,max_input_tokens,max_output_tokens,timeout_ms,created_at,updated_at) values('frozen-run','paper_overview','generating','fixture-generation','fixture-v1','v1','v1',1,?1,1000,1000,5000,'now','now')",[&sha]).unwrap();
        db.execute("insert into ai_evidence_bundles(id,run_id,scope_hash,retrieval_config_version,validation_status,created_at) values('frozen-bundle','frozen-run',?1,'v1','valid','now')",[&sha]).unwrap();
        for (index, quote) in [
            (0, "Exact original page zero."),
            (1, "Unshared page one."),
            (2, "A block spanning both pages."),
        ] {
            let page = if index == 1 { 1 } else { 0 };
            db.execute("insert into source_anchors(id,item_id,attachment_id,content_sha256,page_index,rects_json,quote_exact,quote_prefix,quote_suffix,created_at) values(?1,'paper','frozen-pdf',?2,?3,'[]',?4,'','','now')",params![format!("frozen-{index}"),sha,page,quote]).unwrap();
            db.execute("insert into ai_evidence_items(id,bundle_id,citation_key,ordinal,anchor_id,paper_id,paper_title,attachment_id,content_sha256,page_index,quote_exact,context_text,relation_kind,validation_status,created_at) values(?1,'frozen-bundle',?2,?3,?1,'paper','Frozen','frozen-pdf',?4,?5,?6,?6,'context','valid','now')",params![format!("frozen-{index}"),format!("P1-T{}",index+1),index,sha,page,quote]).unwrap();
        }
        db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values('frozen-parse','paper','frozen-pdf',?1,'fixture','success','success','{}','{}','now','now')",[&sha]).unwrap();
        db.execute("insert into ai_docling_evidence_context_items(evidence_item_id,context_item_id,parse_run_id,raw_ref,item_kind,regions_json,created_at) values('frozen-2','P1-T3','frozen-parse','#/texts/2','text',?1,'now')",[json!([{"pageIndex":0},{"pageIndex":1}]).to_string()]).unwrap();
        let target = crate::structured_resources::find_by_paper(&db, "paper", "portrait").unwrap();
        let denied = resource_grants::grants(
            &db,
            &[Resource::Paper {
                resource_id: "paper".into(),
                attachment_id: Some("frozen-pdf".into()),
                page_index: Some(1),
            }],
            Some(crate::agent::writing_resources::test_sources::source()),
        )
        .unwrap()
        .remove(0)
        .source_id;
        (target, denied)
    };
    let mut owner = tools(
        store,
        conversation,
        vec![
            Resource::Paper {
                resource_id: "paper".into(),
                attachment_id: Some("frozen-pdf".into()),
                page_index: Some(0),
            },
            Resource::StructuredObject {
                resource_id: target.id,
                access: Some(sources::Access::ReadWrite),
            },
        ],
    );
    for source in &mut owner.record.input.context {
        let mut anchor: Value = serde_json::from_str(source.anchor.as_deref().unwrap()).unwrap();
        anchor["sourceSnapshotId"] = json!("frozen-bundle");
        source.anchor = Some(anchor.to_string());
    }
    (owner, target.source_uri, denied)
}

#[tokio::test]
async fn internal_frozen_source_ids_remain_writable_and_page_scopes_cannot_expand() {
    let (_root, store, conversation) = fixture();
    let (owner, target, denied) = frozen_tools(&store, conversation);
    let (_, executor) = registration(
        store.clone(),
        owner.record.clone(),
        OwnedToolRegistration::new(
            vec![],
            Arc::new(crate::agent::composed_tools::RejectingToolExecutor),
        ),
    )
    .into_parts();
    let result = executor
        .authorize_and_execute(call(
            "research_read_source",
            json!({"fileId":"frozen-pdf","view":"structured"}),
        ))
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let result: Value = serde_json::from_str(&result.content).unwrap();
    let body: Value = serde_json::from_str(result["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["snapshotId"], "frozen-bundle");
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    let evidence = &body["items"][0];
    assert_eq!(evidence["evidenceItemId"], "P1-T1");
    assert_eq!(evidence["quoteExact"], "Exact original page zero.");
    let forbidden = executor
        .authorize_and_execute(call(
            "research_read_source",
            json!({"sourceId":denied,"view":"structured"}),
        ))
        .await;
    assert!(forbidden.is_err() || forbidden.unwrap().is_error);
    let read = executor
        .authorize_and_execute(call("research_read_file", json!({"path":target})))
        .await
        .unwrap();
    assert!(!read.is_error, "{}", read.content);
    let claims = [
        ("genre", "原创研究"),
        ("mode", "形式描述"),
        ("paradigm", "计算模型"),
    ]
    .into_iter()
    .map(|(dimension, label)| {
        json!({"id":dimension,"dimension":dimension,"label":label,"description":"仅描述已提供的片段。","certainty":"supported","evidence":[{"evidenceItemId":evidence["evidenceItemId"],"quote":evidence["quoteExact"]}]})
    })
    .collect::<Vec<_>>();
    let portrait=json!({"kind":"paper_portrait_v1","schemaVersion":1,"coverage":"Only the shared first page.","claims":claims,"template":[{"id":"question","title":"问题","question":"回应什么问题？","claimIds":[]}]}).to_string();
    let patch =
        format!("*** Begin Patch\n*** Update File: {target}\n@@\n+{portrait}\n*** End Patch");
    let written = executor
        .authorize_and_execute(call("research_apply_patch", json!({"patch":patch})))
        .await
        .unwrap();
    let written: Value = serde_json::from_str(&written.content).unwrap();
    assert_eq!(written["targets"][0]["status"], "applied", "{written}");
    let db = store.connection().unwrap();
    let source:String=db.query_row("select source_snapshot_id from structured_resource_revisions where resource_id=?1 order by revision desc limit 1",[target.rsplit('/').next().unwrap()],|row|row.get(0)).unwrap();
    assert_eq!(source, "frozen-bundle");
}
