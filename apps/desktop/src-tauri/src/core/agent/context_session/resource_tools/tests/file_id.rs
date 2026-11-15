use super::*;
fn parsed_file(store: &AppStore) {
    let db = store.connection().unwrap();
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values (?1,'not-exported.pdf','application/pdf',1,'now')",["a".repeat(64)]).unwrap();
    db.execute("insert into attachments(id,item_id,content_sha256,display_name,created_at) values ('file','paper',?1,'paper.pdf','now')",["a".repeat(64)]).unwrap();
    let document = json!({"texts":[{"self_ref":"#/texts/0","text":"first page evidence","prov":[{"page_no":1}]},{"self_ref":"#/texts/1","text":"second page evidence","prov":[{"page_no":2}]}]});
    db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values ('parse0','paper','file',?1,'fixture','success','success',?2,'{}','now','now')",params!["a".repeat(64),document.to_string()]).unwrap();
}
fn file(page_index: Option<u32>) -> Resource {
    Resource::Paper {
        resource_id: "paper".into(),
        attachment_id: Some("file".into()),
        page_index,
    }
}
#[test]
fn one_file_id_reads_and_searches_current_database_content_without_exporting_or_reparsing() {
    let (_root, store, id) = fixture();
    parsed_file(&store);
    let tools = tools(&store, id, vec![file(None)]);
    let list = tools
        .evaluate(&call("research_list_sources", json!({})))
        .unwrap();
    assert_eq!(list["sources"][0]["fileId"], "file");
    assert_eq!(
        list["sources"][0]["sourceUrl"],
        "source://library/paper/file"
    );
    let source = list["sources"][0]["sourceId"].clone();
    let read = call("research_read_source", json!({"fileId":"file"}));
    tools.validate(&read).unwrap();
    let first = tools.evaluate(&read).unwrap();
    assert_eq!(first["fileId"], "file");
    assert_eq!(first["sourceUrl"], "source://library/paper/file");
    assert_eq!(first["sourceId"], source);
    assert_eq!(first["name"], list["sources"][0]["name"]);
    assert_eq!(first["startLine"], 1);
    assert!(first["endLine"].as_u64().is_some());
    assert_eq!(first["readViewId"].as_str().unwrap().len(), 64);
    assert!(
        first["text"]
            .as_str()
            .unwrap()
            .contains("first page evidence")
    );
    let query = call(
        "research_search_sources",
        json!({"fileId":"file","query":"second page"}),
    );
    tools.validate(&query).unwrap();
    let found = tools.evaluate(&query).unwrap();
    assert_eq!(found["total"], 1);
    assert_eq!(found["matches"][0]["sourceId"], source);
    assert_eq!(
        found["matches"][0]["sourceUrl"],
        "source://library/paper/file"
    );
    assert_eq!(found["scope"]["sourceUrl"], "source://library/paper/file");
    assert_eq!(found["scope"]["name"], first["name"]);
    let empty = tools
        .evaluate(&call(
            "research_search_sources",
            json!({"fileId":"file","query":"not present"}),
        ))
        .unwrap();
    assert_eq!(empty["total"], 0);
    assert_eq!(empty["scope"], found["scope"]);
    let by_source = tools
        .evaluate(&call(
            "research_search_sources",
            json!({"sourceId":source,"query":"not present"}),
        ))
        .unwrap();
    assert_eq!(by_source["scope"], found["scope"]);
    let global = tools
        .evaluate(&call(
            "research_search_sources",
            json!({"query":"second page"}),
        ))
        .unwrap();
    assert!(global.get("scope").is_none());
    {
        let db = store.connection().unwrap();
        db.execute("insert into docling_parse_runs(id,item_id,attachment_id,content_sha256,service_base_url,docling_status,parse_status,json_content,raw_response,created_at,finished_at) values ('parse1','paper','file',?1,'fixture','success','success',?2,'{}','now','now')",params!["a".repeat(64),json!({"texts":[{"text":"latest saved evidence"}]}).to_string()]).unwrap();
    }
    let latest = tools.evaluate(&read).unwrap();
    assert_eq!(latest["sourceId"], source);
    assert_eq!(latest["fileId"], "file");
    assert!(
        latest["text"]
            .as_str()
            .unwrap()
            .contains("latest saved evidence")
    );
    assert!(
        !store
            .agent_workspace_directory()
            .join(".docling-cache")
            .exists()
    );
    let db = store.connection().unwrap();
    let count: i64 = db
        .query_row("select count(*) from docling_parse_runs", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 2);
}
#[test]
fn file_id_cannot_bypass_page_scopes_exclusions_or_current_attachment_version() {
    let (_root, store, id) = fixture();
    parsed_file(&store);
    let tools = tools(&store, id, vec![file(Some(0))]);
    let text = tools
        .evaluate(&call("research_read_source", json!({"fileId":"file"})))
        .unwrap();
    assert!(
        text["text"]
            .as_str()
            .unwrap()
            .contains("first page evidence")
    );
    assert!(
        !text["text"]
            .as_str()
            .unwrap()
            .contains("second page evidence")
    );
    assert_eq!(
        tools.evaluate(&call("research_read_source", json!({"fileId":"foreign"}))),
        Err(Error::SourceUnavailable)
    );
    let excluded = super::tools(
        &store,
        id,
        vec![Resource::PaperGroup {
            resource_id: "paper".into(),
            new_member_access: Some(sources::Access::ReadOnly),
            group_access: None,
            parts: vec![sources::Part {
                resource: file(None),
                label: "Excluded file".into(),
                selected: false,
                access: sources::Access::ReadOnly,
            }],
        }],
    );
    let list = excluded
        .evaluate(&call("research_list_sources", json!({})))
        .unwrap();
    assert!(
        list["sources"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["fileId"] != "file")
    );
    for request in [
        call("research_read_source", json!({"fileId":"file"})),
        call(
            "research_search_sources",
            json!({"fileId":"file","query":"evidence"}),
        ),
    ] {
        excluded.validate(&request).unwrap();
        assert_eq!(excluded.evaluate(&request), Err(Error::SourceUnavailable));
    }
    let db = store.connection().unwrap();
    db.execute("insert into content_objects(sha256,relative_path,mime_type,byte_size,created_at) values (?1,'changed.pdf','application/pdf',1,'now')",["b".repeat(64)]).unwrap();
    db.execute(
        "update attachments set content_sha256=?1 where id='file'",
        ["b".repeat(64)],
    )
    .unwrap();
    drop(db);
    assert_eq!(
        tools.evaluate(&call("research_read_source", json!({"fileId":"file"}))),
        Err(Error::PaperNotParsed)
    );
}
#[test]
fn ambiguous_file_scopes_require_an_exact_source_handle_and_arguments_stay_recoverable() {
    let (_root, store, id) = fixture();
    parsed_file(&store);
    let tools = tools(&store, id, vec![file(Some(0)), file(Some(1))]);
    assert_eq!(
        tools.evaluate(&call("research_read_source", json!({"fileId":"file"}))),
        Err(Error::InvalidInput)
    );
    let rows = tools
        .evaluate(&call("research_list_sources", json!({})))
        .unwrap();
    let scoped = tools
        .evaluate(&call(
            "research_read_source",
            json!({"sourceId":rows["sources"][0]["sourceId"]}),
        ))
        .unwrap();
    assert!(scoped["text"].is_string());
    for args in [
        json!({}),
        json!({"fileId":""}),
        json!({"fileId":"file","sourceId":"other"}),
        json!({"fileId":"file","maxChars":13500}),
    ] {
        assert!(matches!(
            tools.validate(&call("research_read_source", args)),
            Err(RunError::ToolArguments(_))
        ));
    }
}
