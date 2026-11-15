//! Pause only the real host's first materialization request, after SQL settles.
use super::*;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicBool, Ordering},
};

pub(super) struct Pause {
    entered: PathBuf,
    release: PathBuf,
}
impl Pause {
    pub(super) async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while !self.entered.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the actual host must reach materialization");
    }
    pub(super) fn release(&self) {
        fs::write(&self.release, b"resume").unwrap();
    }
}
impl Drop for Pause {
    fn drop(&mut self) {
        let _ = fs::write(&self.release, b"resume");
    }
}

pub(super) fn paused_binding(root: &Path) -> (Binding, Pause) {
    paused_binding_for(root, "context")
}
pub(super) fn paused_host_files_binding(root: &Path) -> (Binding, Pause) {
    paused_binding_for(root, "hostFiles")
}
fn paused_binding_for(root: &Path, operation: &str) -> (Binding, Pause) {
    fn copy(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                copy(&entry.path(), &target.join(entry.file_name()));
            } else {
                fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
            }
        }
    }
    let source = packages().join("writing_projects");
    let target = root.join("paused-packages/writing_projects");
    copy(&source, &target);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(target.join("manifest.json")).unwrap()).unwrap();
    let entry = manifest["native"]["entry"].as_str().unwrap();
    let actual = source.join(entry);
    let executable = target.join(entry);
    let pause = Pause {
        entered: root.join("native-source-entered"),
        release: root.join("native-source-release"),
    };
    let node = std::process::Command::new("node")
        .args(["-p", "process.execPath"])
        .output()
        .unwrap();
    assert!(node.status.success());
    let node = String::from_utf8(node.stdout).unwrap();
    let quote = |path: &Path| serde_json::to_string(path).unwrap();
    let script = format!(
        "#!{}\nconst entry = {};\nconst entered = {};\nconst release = {};\nconst operation = {};\n{}",
        node.trim(),
        quote(&actual),
        quote(&pause.entered),
        quote(&pause.release),
        serde_json::to_string(operation).unwrap(),
        HOST
    );
    fs::write(&executable, script).unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
    (binding(root.join("paused-packages")), pause)
}
const HOST: &str = r#"
const fs = require('node:fs');
const {spawn} = require('node:child_process');
const input = require('node:readline').createInterface({input:process.stdin});
const child = spawn(entry, process.argv.slice(2), {stdio:['pipe','pipe','pipe']});
child.stdout.pipe(process.stdout);
child.stderr.pipe(process.stderr);
child.once('close', code => {
  process.exitCode = code ?? 1;
  input.close();
  process.stdin.destroy();
});
(async () => {
  let paused=false;
  for await (const line of input) {
    const frame=JSON.parse(line);
    if (!paused && frame.body.kind==='projects' && frame.body.operation===operation) {
      const request=JSON.parse(fs.readFileSync(frame.body.snapshot.relativeHandle, 'utf8'));
      if (operation==='hostFiles' || request.input.phase==='materialize') {
        paused = true;
        fs.writeFileSync(entered, 'entered');
        while (!fs.existsSync(release)) await new Promise(resolve => setTimeout(resolve, 10));
      }
    }
    child.stdin.write(line+'\n');
  }
  child.stdin.end();
})().catch(error => { console.error(error); process.exit(1); });
"#;

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_context_rechecks_changed_file_after_native_wait_without_holding_sql() {
    let f = fixture(true);
    let (binding, pause) = paused_binding(f.root.path());
    let service =
        WritingSubmissionService::from_app_store(&f.store).with_runtime_context(binding.clone());
    let input = request(&f, Uuid::new_v4());
    let prepared = tokio::spawn({
        let models = f.models.clone();
        async move { service.prepare(&models, input).await }
    });
    pause.entered().await;
    let store = f.store.clone();
    tokio::time::timeout(
        Duration::from_secs(3),
        tokio::task::spawn_blocking(move || {
            let db = store.connection().unwrap();
            assert!(db.is_autocommit());
            fs::write(
                store
                    .manuscript_directory()
                    .join(f.manuscript.to_string())
                    .join("source/main.tex"),
                "Changed while Native waited",
            )
            .unwrap();
        }),
    )
    .await
    .expect("Native wait must release SQL")
    .unwrap();
    pause.release();
    let result = prepared.await.unwrap();
    assert!(
        matches!(result, Err(WritingSubmissionError::Storage(cause)) if cause.as_inner().diagnostic().is_some_and(|detail| detail.safe_cause == "writing_context_changed"))
    );
    assert_eq!(count(&f), 0);
    binding.service.shutdown().await.unwrap();
}

#[tokio::test]
#[ignore = "requires an independently built writing v2 package; run the explicit C05 package acceptance"]
async fn actual_package_context_waiter_cancellation_retains_original_owner_until_one_commit() {
    struct Lease(Arc<AtomicBool>);
    impl Drop for Lease {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let f = fixture(true);
    let (binding, pause) = paused_binding(f.root.path());
    let released = Arc::new(AtomicBool::new(false));
    let mut service =
        WritingSubmissionService::from_app_store(&f.store).with_runtime_context(binding.clone());
    service.worker_retention =
        crate::agent::repository::WorkerRetention::new(Lease(Arc::clone(&released)));
    let input = request(&f, Uuid::new_v4());
    let retry = input.clone();
    let prepared = tokio::spawn({
        let models = f.models.clone();
        async move { service.prepare(&models, input).await }
    });
    pause.entered().await;
    prepared.abort();
    assert!(matches!(prepared.await, Err(error) if error.is_cancelled()));
    assert!(!released.load(Ordering::SeqCst));
    pause.release();
    tokio::time::timeout(Duration::from_secs(20), async {
        while !released.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(count(&f), 1);
    let service =
        WritingSubmissionService::from_app_store(&f.store).with_runtime_context(binding.clone());
    service.prepare(&f.models, retry).await.unwrap();
    assert_eq!(count(&f), 1);
    binding.service.shutdown().await.unwrap();
}
