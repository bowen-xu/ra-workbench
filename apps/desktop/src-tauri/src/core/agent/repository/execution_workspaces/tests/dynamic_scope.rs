//! Primary dynamic authority is distinct from the frozen files it authorizes.
use super::*;
use crate::agent::context_session::{
    resource_grants,
    sources::{Access, Resource},
};
use serde_json::{Value, json};

fn with_grant() -> (Connection, Uuid, Uuid, WorkspaceManifest, SubmissionInput) {
    let mut db = Connection::open_in_memory().unwrap();
    let (request, manuscript, manifest) = setup(&mut db);
    let mut input = crate::agent::repository::submissions::read(
        &db,
        manifest.identity.conversation_id,
        request,
    )
    .unwrap()
    .unwrap()
    .input;
    let resource = Resource::Manuscript {
        resource_id: manuscript.to_string(),
        new_member_access: Some(Access::ReadWrite),
        group_access: None,
        parts: vec![],
    };
    let mut grant = resource_grants::grants(
        &db,
        std::slice::from_ref(&resource),
        Some(crate::agent::writing_resources::test_sources::source()),
    )
    .unwrap()
    .remove(0);
    let mut anchor: Value = serde_json::from_str(grant.anchor.as_deref().unwrap()).unwrap();
    anchor["contextOwner"] = json!({"kind":"manuscript","resourceId":manuscript});
    anchor["contextScope"] = serde_json::to_value(resource).unwrap();
    anchor["supplementalContext"] = false.into();
    grant.anchor = Some(anchor.to_string());
    input.context.push(grant);
    (db, request, manuscript, manifest, input)
}
fn save(db: &Connection, input: &SubmissionInput) {
    db.execute(
        "update agent_submissions set input_json=?1",
        [serde_json::to_string(input).unwrap()],
    )
    .unwrap();
}

#[test]
fn primary_dynamic_grant_registers_the_same_manifest_without_implying_approval() {
    let (mut db, request, manuscript, manifest, input) = with_grant();
    save(&db, &input);
    let tx = db.transaction().unwrap();
    let binding = prepare_in(&tx, request, manuscript, &manifest).unwrap();
    assert_eq!(
        prepare_in(&tx, request, manuscript, &manifest).unwrap(),
        binding
    );
    tx.commit().unwrap();
    assert_eq!(authorize(&db, &binding, &manifest), Err(Error::Denied));
    let tx = db.transaction().unwrap();
    approve_in(&tx, &binding).unwrap();
    tx.commit().unwrap();
    authorize(&db, &binding, &manifest).unwrap();
    let mut changed = manifest.clone();
    changed.identity.lease_id = Uuid::new_v4();
    assert_eq!(authorize(&db, &binding, &changed), Err(Error::Conflict));
}

#[test]
fn foreign_incomplete_or_supplemental_grants_do_not_bypass_file_validation() {
    let (mut db, request, manuscript, manifest, input) = with_grant();
    let grant = input.context.last().unwrap();
    let original: Value = serde_json::from_str(grant.anchor.as_deref().unwrap()).unwrap();
    let foreign = Uuid::new_v4().to_string();
    for pointer in [
        "/content",
        "/manuscriptId",
        "/contextOwner/kind",
        "/contextOwner/resourceId",
        "/resource/resourceId",
        "/contextScope/resourceId",
        "/supplementalContext",
    ] {
        let mut candidate = input.clone();
        let mut anchor = original.clone();
        *anchor.pointer_mut(pointer).unwrap() = if pointer == "/supplementalContext" {
            true.into()
        } else {
            foreign.clone().into()
        };
        candidate.context.last_mut().unwrap().anchor = Some(anchor.to_string());
        save(&db, &candidate);
        let tx = db.transaction().unwrap();
        assert_eq!(
            prepare_in(&tx, request, manuscript, &manifest),
            Err(Error::Conflict),
            "{pointer}"
        );
        tx.rollback().unwrap();
    }
    let mut candidate = input;
    candidate.context.last_mut().unwrap().resource_id = "main.tex".into();
    save(&db, &candidate);
    let tx = db.transaction().unwrap();
    assert_eq!(
        prepare_in(&tx, request, manuscript, &manifest),
        Err(Error::Conflict)
    );
}

#[test]
fn a_valid_directory_grant_does_not_relax_any_frozen_file_version() {
    let (mut db, request, manuscript, manifest, input) = with_grant();
    for pointer in ["revision", "diskSha256", "bufferRevision"] {
        let mut candidate = input.clone();
        let source = &mut candidate.context[0];
        if pointer == "revision" {
            source.revision = format!("sha256:{}", "f".repeat(64));
        } else {
            let mut anchor: Value =
                serde_json::from_str(source.anchor.as_deref().unwrap()).unwrap();
            anchor[pointer] = "changed".into();
            source.anchor = Some(anchor.to_string());
        }
        save(&db, &candidate);
        let tx = db.transaction().unwrap();
        assert_eq!(
            prepare_in(&tx, request, manuscript, &manifest),
            Err(Error::Conflict),
            "{pointer}"
        );
        tx.rollback().unwrap();
    }
}
