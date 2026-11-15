use super::*;
use crate::agent::context_session::sources::{Access, Part};

fn scope(excluded: Vec<String>) -> Resource {
    Resource::PaperGroup {
        resource_id: "paper".into(),
        new_member_access: Some(Access::ReadOnly),
        group_access: None,
        parts: vec![Part {
            resource: Resource::PaperNotes {
                resource_id: "paper".into(),
                excluded_note_ids: excluded,
            },
            label: "Notes".into(),
            selected: true,
            access: Access::ReadOnly,
        }],
    }
}
#[test]
fn thousands_of_resources_are_found_without_bodies_or_inventory_in_the_result() {
    let (_root, store, id) = fixture();
    let mut excluded = None;
    for index in 0..3000 {
        let page = note(&store, "Private body must not enter a directory result");
        store
            .connection()
            .unwrap()
            .execute(
                "update note_pages set title=?1 where id=?2",
                params![format!("Method {index:04}.md"), page],
            )
            .unwrap();
        if index == 1234 {
            excluded = Some(page);
        }
    }
    let tools = tools(&store, id, vec![scope(vec![excluded.unwrap()])]);
    let list = tools
        .evaluate(&call(
            "research_list_sources",
            json!({"resourceId":"paper"}),
        ))
        .unwrap();
    assert_eq!(list["total"], 3003); // 2999 readable notes, metadata, three structured objects.
    assert_eq!(list["sources"].as_array().unwrap().len(), 20);
    assert_eq!(list["nextOffset"], 20);
    assert!(list.to_string().len() <= 32 * 1024);
    assert!(!list.to_string().contains("Private body"));
    assert!(!list.to_string().contains("anchor"));
    let found=tools.evaluate(&call("research_find_sources",json!({"resourceId":"paper","nameQuery":"METHOD 123","pathGlob":"*.MD","resourceType":"note","limit":2}))).unwrap();
    assert_eq!(found["total"], 9);
    assert_eq!(found["sources"].as_array().unwrap().len(), 2);
    assert_eq!(found["sources"][0]["name"], "Method 1230.md");
    assert_eq!(found["nextOffset"], 2);
    let end = tools
        .evaluate(&call(
            "research_find_sources",
            json!({"resourceId":"paper","nameQuery":"method 123","offset":8}),
        ))
        .unwrap();
    assert_eq!(end["sources"].as_array().unwrap().len(), 1);
    assert!(end["nextOffset"].is_null());
    let source = found["sources"][0]["sourceId"].clone();
    assert!(
        tools
            .evaluate(&call("research_read_source", json!({"sourceId":source})))
            .unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("Private body")
    );
    assert_eq!(
        tools.evaluate(&call(
            "research_find_sources",
            json!({"resourceId":"foreign","nameQuery":"Method"})
        )),
        Err(Error::SourceUnavailable)
    );
}
#[test]
fn name_search_and_filtered_content_search_use_current_titles_and_exact_authority() {
    let (_root, store, id) = fixture();
    let page = note(&store, "needle current evidence");
    let excluded = note(&store, "needle forbidden evidence");
    store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set title='Methods.md' where id=?1",
            [&page],
        )
        .unwrap();
    let tools = tools(&store, id, vec![scope(vec![excluded])]);
    let args = json!({"resourceId":"paper","nameQuery":"Methods","resourceType":"note"});
    tools
        .validate(&call("research_find_sources", args.clone()))
        .unwrap();
    let found = tools
        .evaluate(&call("research_find_sources", args))
        .unwrap();
    assert_eq!(found["total"], 1);
    let args =
        json!({"query":"needle","resourceId":"paper","nameQuery":"Methods","resourceType":"note"});
    tools
        .validate(&call("research_search_sources", args.clone()))
        .unwrap();
    let result = tools
        .evaluate(&call("research_search_sources", args))
        .unwrap();
    assert_eq!(result["total"], 1);
    assert!(!result.to_string().contains("forbidden"));
    store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set title='Results.md' where id=?1",
            [&page],
        )
        .unwrap();
    assert_eq!(
        tools
            .evaluate(&call(
                "research_find_sources",
                json!({"nameQuery":"Methods"})
            ))
            .unwrap()["total"],
        0
    );
    let current = tools
        .evaluate(&call("research_find_sources", json!({"pathGlob":"*.md"})))
        .unwrap();
    assert_eq!(current["sources"][0]["name"], "Results.md");
    store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set status='trashed' where id=?1",
            [&page],
        )
        .unwrap();
    assert_eq!(
        tools
            .evaluate(&call("research_find_sources", json!({"pathGlob":"*.md"})))
            .unwrap()["total"],
        0
    );
}
#[test]
fn invalid_discovery_arguments_are_recoverable_and_unicode_names_are_preserved() {
    let (_root, store, id) = fixture();
    let page = note(&store, "not listed");
    store
        .connection()
        .unwrap()
        .execute(
            "update note_pages set title=?1 where id=?2",
            params!["长".repeat(240), page],
        )
        .unwrap();
    let tools = tools(&store, id, vec![scope(vec![])]);
    for args in [
        json!({}),
        json!({"nameQuery":""}),
        json!({"nameQuery":"test","limit":2000}),
        json!({"pathGlob":"../*"}),
        json!({"nameQuery":"x","unknown":true}),
        json!({"resourceType":"secret"}),
    ] {
        assert!(matches!(
            tools.validate(&call("research_find_sources", args)),
            Err(RunError::ToolArguments(_))
        ));
    }
    let result = tools
        .evaluate(&call(
            "research_find_sources",
            json!({"resourceType":"note"}),
        ))
        .unwrap();
    assert_eq!(
        result["sources"][0]["name"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        240
    );
    assert_eq!(result["sources"][0]["nameTruncated"], false);
}

#[test]
fn project_paths_are_found_with_permissions_and_byte_bound_pagination_without_file_reads() {
    let (_root, store, id) = fixture();
    let project = Uuid::new_v4();
    let long_prefix = format!("{}/", "x".repeat(90)).repeat(10);
    {
        let db = store.connection().unwrap();
        db.execute("insert into research_projects(id,title,summary,created_at) values ('project','Fixture','','now')",[]).unwrap();
        db.execute("insert into manuscripts(id,project_id,title,root_path,created_at,updated_at) values (?1,'project','Fixture','main.tex','now','now')",[project.to_string()]).unwrap();
        let mut insert=db.prepare("insert into manuscript_files(manuscript_id,relative_path,file_kind,byte_size,sha256,modified_at) values (?1,?2,'tex',1,?3,'now')").unwrap();
        for index in 0..1200 {
            insert
                .execute(params![
                    project.to_string(),
                    format!("sections/methods-{index:04}.tex"),
                    "b".repeat(64)
                ])
                .unwrap();
        }
        for index in 0..20 {
            insert
                .execute(params![
                    project.to_string(),
                    format!("{long_prefix}long-{index:02}.tex"),
                    "b".repeat(64)
                ])
                .unwrap();
        }
    }
    let file = |path: &str| Resource::ManuscriptFile {
        resource_id: project,
        path: path.into(),
    };
    let scope = Resource::Manuscript {
        resource_id: project.to_string(),
        new_member_access: Some(Access::ReadWrite),
        group_access: None,
        parts: vec![
            Part {
                resource: file("sections/methods-0000.tex"),
                label: "Denied".into(),
                access: Access::ReadOnly,
                selected: false,
            },
            Part {
                resource: file("sections/methods-0001.tex"),
                label: "Read only".into(),
                access: Access::ReadOnly,
                selected: true,
            },
        ],
    };
    let tools = tools(&store, id, vec![scope]);
    let result=tools.evaluate(&call("research_find_sources",json!({"resourceId":project.to_string(),"pathGlob":"sections/**/*.tex","nameQuery":"methods-000"}))).unwrap();
    assert_eq!(result["total"], 9);
    assert_eq!(result["sources"][0]["path"], "sections/methods-0001.tex");
    assert_eq!(result["sources"][0]["access"], "readOnly");
    assert_eq!(result["sources"][1]["access"], "readWrite");
    let mut offset = 0;
    let mut count = 0;
    loop {
        let result = tools
            .evaluate(&call(
                "research_find_sources",
                json!({"resourceId":project.to_string(),"nameQuery":"long-","offset":offset}),
            ))
            .unwrap();
        assert_eq!(result["total"], 20);
        assert!(result.to_string().len() <= 32 * 1024);
        let rows = result["sources"].as_array().unwrap();
        assert!(!rows.is_empty() && rows.len() < 20);
        assert_eq!(rows[0]["nameTruncated"], true);
        assert!(rows[0]["path"].as_str().unwrap().len() > 256);
        assert!(rows[0].get("anchor").is_none());
        count += rows.len();
        let Some(next) = result["nextOffset"].as_u64() else {
            break;
        };
        assert!(next > offset);
        offset = next;
    }
    assert_eq!(count, 20);
}
