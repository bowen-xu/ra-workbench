use super::*;
use crate::agent::context_session::sources::{Access, Part};

#[test]
fn item_id_grant_does_not_grow_with_inventory_and_tools_discover_readable_notes() {
    let (_root, store, id) = fixture();
    let mut scope = Resource::PaperGroup {
        resource_id: "paper".into(),
        parts: vec![],
        new_member_access: Some(Access::ReadOnly),
        group_access: None,
    };
    let initial = resource_grants::grants(
        &store.connection().unwrap(),
        std::slice::from_ref(&scope),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    let mut notes = Vec::new();
    for index in 0..120 {
        let page = note(&store, &format!("body {index}"));
        if let Resource::PaperGroup { parts, .. } = &mut scope {
            parts.push(Part {
                resource: Resource::Note {
                    resource_id: Uuid::parse_str(&page).unwrap(),
                },
                label: format!("Inventory note {index}"),
                selected: false,
                access: Access::ReadOnly,
            });
        }
        notes.push(page);
    }
    let expanded = resource_grants::grants(
        &store.connection().unwrap(),
        std::slice::from_ref(&scope),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap();
    assert_eq!(
        initial, expanded,
        "adding inventory never changes authority or its identity"
    );
    let anchor = expanded[0].anchor.as_ref().unwrap();
    assert!(anchor.len() < 1024);
    assert!(!anchor.contains("Inventory"));
    if let Resource::PaperGroup { parts, .. } = &mut scope {
        parts.push(Part {
            resource: Resource::PaperNotes {
                resource_id: "paper".into(),
                excluded_note_ids: vec![notes[0].clone()],
            },
            label: "Reading notes".into(),
            selected: true,
            access: Access::ReadOnly,
        });
    }
    let tools = tools(&store, id, vec![scope]); // Goes through the actual submission validator.
    let mut offset = 0;
    let mut listed = Vec::new();
    loop {
        let args = json!({"resourceId":"paper","offset":offset});
        tools
            .validate(&call("research_list_sources", args.clone()))
            .unwrap();
        let result = tools
            .evaluate(&call("research_list_sources", args))
            .unwrap();
        listed.extend(result["sources"].as_array().unwrap().iter().cloned());
        let Some(next) = result["nextOffset"].as_u64() else {
            break;
        };
        offset = next;
    }
    assert_eq!(listed.len(), 123); // 119 notes + metadata + three structured objects.
    assert!(
        listed
            .iter()
            .all(|row| row["ownerResourceId"] == "paper" && row["access"] == "readOnly")
    );
    assert!(!listed.iter().any(|row| row["resourceId"] == notes[0]));
    assert_eq!(
        tools.evaluate(&call(
            "research_list_sources",
            json!({"resourceId":"foreign"})
        )),
        Err(Error::SourceUnavailable)
    );
    let next = note(&store, "later body");
    let list = tools
        .evaluate(&call(
            "research_list_sources",
            json!({"resourceId":"paper"}),
        ))
        .unwrap();
    assert_eq!(list["total"], 124);
    assert!(
        tools
            .entries(&store.connection().unwrap())
            .unwrap()
            .iter()
            .any(|entry| entry.identity.resource_id == next)
    );
}

#[test]
fn listing_reflects_note_permission_overrides_without_publishing_unreadable_members() {
    let (_root, store, id) = fixture();
    let kept = note(&store, "kept");
    let excluded = note(&store, "excluded");
    let scope = Resource::PaperGroup {
        resource_id: "paper".into(),
        new_member_access: Some(Access::ReadWrite),
        group_access: None,
        parts: vec![
            Part {
                resource: Resource::PaperNotes {
                    resource_id: "paper".into(),
                    excluded_note_ids: vec![excluded.clone()],
                },
                label: "Notes".into(),
                selected: true,
                access: Access::ReadWrite,
            },
            Part {
                resource: Resource::Note {
                    resource_id: Uuid::parse_str(&kept).unwrap(),
                },
                label: "Old name".into(),
                selected: true,
                access: Access::ReadOnly,
            },
        ],
    };
    let tools = tools(&store, id, vec![scope]);
    let list = tools
        .evaluate(&call(
            "research_list_sources",
            json!({"resourceId":"paper"}),
        ))
        .unwrap();
    let rows = list["sources"].as_array().unwrap();
    assert!(!rows.iter().any(|row| row["resourceId"] == excluded));
    assert_eq!(
        rows.iter().find(|row| row["resourceId"] == kept).unwrap()["access"],
        "readOnly"
    );
    assert!(rows.iter().any(|row| row["access"] == "readWrite"));
}
