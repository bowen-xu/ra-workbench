use super::*;
use std::io::Write;

fn record_detection(checked_at: String, home: &Path) -> LatexToolchainInfo {
    assert_eq!(checked_at.len(), 24);
    assert!(checked_at.ends_with('Z'));
    std::fs::create_dir_all(home).unwrap();
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(home.join("detection-calls"))
        .unwrap();
    writeln!(log, "{checked_at}").unwrap();
    LatexToolchainInfo {
        available: false,
        tools: vec![],
        supported_engines: vec!["pdflatex".into()],
        checked_at,
    }
}

#[tokio::test]
async fn authorization_then_detection_then_root_precede_unavailable_toolchain() {
    let mut h = Harness::new().await;
    h.compiler.toolchain = None;
    h.compiler.compiler_source = Some(WritingWorkspaceCompileSource {
        detect: record_detection,
        ..h.compiler.compiler_source.unwrap()
    });
    let marker = h
        .fixture
        .store
        .writing_cache_directory()
        .join("toolchain-home/detection-calls");
    let mut foreign = h.compiler.clone();
    foreign.binding.turn_id = Uuid::new_v4();
    assert!(matches!(
        foreign
            .authorize_and_execute(h.compile_call("foreign-before-detect"))
            .await,
        Err(RunError::ToolRejected)
    ));
    assert!(!marker.exists());
    let missing = h.call(
        "missing-root",
        COMPILE_WORKSPACE,
        json!({"rootPath":"missing.tex","engine":"pdflatex"}),
    );
    assert!(matches!(
        h.compiler.authorize_and_execute(missing).await,
        Err(RunError::ToolRejected)
    ));
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 1);
    assert!(matches!(
        h.compiler
            .authorize_and_execute(h.compile_call("unavailable-after-root"))
            .await,
        Err(RunError::ToolUnavailable)
    ));
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 2);
    assert!(h.outputs().is_empty());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}

#[tokio::test]
async fn absent_source_keeps_authorization_and_refuses_without_launching() {
    let mut h = Harness::new().await;
    h.compiler.compiler_source = None;
    let mut foreign = h.compiler.clone();
    foreign.binding.turn_id = Uuid::new_v4();
    assert!(matches!(
        foreign
            .authorize_and_execute(h.compile_call("foreign-no-source"))
            .await,
        Err(RunError::ToolRejected)
    ));
    assert!(matches!(
        h.compiler
            .authorize_and_execute(h.compile_call("missing-source"))
            .await,
        Err(RunError::ToolUnavailable)
    ));
    assert!(h.outputs().is_empty());
    assert_eq!(h.original(), "Frozen manuscript");
    h.sessions.processes().shutdown().await.unwrap();
}
