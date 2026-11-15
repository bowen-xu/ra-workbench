//! Original project inventory feeds the production matrix's real source choices.
use super::evidence_matrix::{close, counts, fixture};
use super::*;
use crate::runtime_plugins::literature_evidence::{
    EvidenceReadRequest as Request, EvidenceReadResult as Result,
};

async fn read(
    f: &Fixture,
    request: Request,
) -> crate::runtime_plugins::native::NativeResult<Result> {
    f.service
        .evidence_read(f.store.shared_agent_connection(), f.key.clone(), request)
        .await
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_inventory_preserves_scope_archival_and_linked_history() {
    let f = fixture().await;
    f.store.connection().unwrap().execute_batch("insert into claims(id,project_id,statement,origin,provenance_kind,epistemic_role,revision,created_at,updated_at) values('matrix-claim','matrix-project','Synthetic project claim','user','human_original','assertion',1,'before','before'),('global-claim',null,'Synthetic global claim','user','human_original','question',1,'before','before');
    insert into evidence_links(id,claim_id,anchor_id,stance,rationale,created_at) values('matrix-link','matrix-claim','matrix-first-anchor','supports','Synthetic rationale','before');").unwrap();
    let Result::Claims(claims) = read(
        &f,
        Request::Claims {
            project_id: Some("matrix-project".into()),
        },
    )
    .await
    .unwrap() else {
        panic!("wrong claims")
    };
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].id, "matrix-claim");
    assert_eq!(claims[0].evidence_count, 1);
    assert_eq!(claims[0].project_title.as_deref(), Some("Synthetic matrix"));
    let Result::Claims(all) = read(&f, Request::Claims { project_id: None })
        .await
        .unwrap()
    else {
        panic!("wrong claims")
    };
    assert_eq!(all.len(), 2);
    let Result::Sources(sources) = read(
        &f,
        Request::Sources {
            project_id: Some("matrix-project".into()),
        },
    )
    .await
    .unwrap() else {
        panic!("wrong sources")
    };
    assert_eq!(sources.len(), 2);
    assert_eq!(sources[0].anchor_id, "matrix-second-anchor");
    f.store
        .connection()
        .unwrap()
        .execute(
            "update bibliographic_items set archived_at='now' where id='paper'",
            [],
        )
        .unwrap();
    let Result::Sources(sources) = read(
        &f,
        Request::Sources {
            project_id: Some("matrix-project".into()),
        },
    )
    .await
    .unwrap() else {
        panic!("wrong sources")
    };
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].item_id, "matrix-second");
    let Result::Links(links) = read(
        &f,
        Request::Links {
            claim_id: "matrix-claim".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("wrong links")
    };
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].source.anchor_id, "matrix-first-anchor");
    assert_eq!(links[0].source.quote_exact, "Synthetic first result");
    assert!(
        f.service.host.try_lock().unwrap().is_none(),
        "fixed reads must not start a needless child process"
    );
    let f = reopen_actual_fixture(f, false).await;
    let Result::Links(reopened) = read(
        &f,
        Request::Links {
            claim_id: "matrix-claim".into(),
        },
    )
    .await
    .unwrap() else {
        panic!("wrong links")
    };
    assert_eq!(reopened, links);
    close(f).await;
}

#[tokio::test]
#[ignore = "requires the current independent literature package"]
async fn actual_package_matrix_inventory_sql_failure_retains_safe_identity_and_a_usable_owner() {
    use rusqlite::hooks::{AuthAction, AuthContext, Authorization};
    let f = fixture().await;
    f.store
        .connection()
        .unwrap()
        .authorizer(Some(|context: AuthContext<'_>| match context.action {
            AuthAction::Read {
                table_name: "source_anchors",
                ..
            } => Authorization::Deny,
            _ => Authorization::Allow,
        }));
    let error = read(
        &f,
        Request::Sources {
            project_id: Some("matrix-project".into()),
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code, "storage_error");
    assert_eq!(error.package_key.as_deref(), Some(f.key.as_str()));
    assert!(error.operation_id.is_some());
    assert!(error.instance_id.is_none() && error.request_id.is_none());
    assert!(!error.message.contains("Synthetic first result"));
    f.store
        .connection()
        .unwrap()
        .authorizer(None::<fn(AuthContext<'_>) -> Authorization>);
    let Result::Sources(sources) = read(&f, Request::Sources { project_id: None })
        .await
        .unwrap()
    else {
        panic!("wrong sources")
    };
    assert_eq!(sources.len(), 2);
    assert_eq!(counts(&f), (0, 0, 0, 0));
    close(f).await;
}
