//! Exercise aggregate cancellation against actual owned processes, not mocks.
use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Events(Mutex<Vec<(Uuid, CompactionHookNotification)>>);
impl HookHandlerObserver for Events {
    fn notify(&self, id: Uuid, _: &CompactionHookRequest, event: CompactionHookNotification) {
        self.0.lock().unwrap().push((id, event));
    }
}

#[tokio::test]
async fn canceling_concurrent_native_hooks_kills_every_pending_process_once() {
    let roots = [tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap()];
    let events = Arc::new(Events::default());
    let mut handlers = vec![CompactionHookHandler {
        id: Uuid::from_u128(10),
        stage: CompactionHookStage::SessionStartCompact,
        hooks: Arc::new(hooks(
            roots[0].path(),
            "cat >/dev/null; printf 'completed context'",
            1024,
            Duration::from_secs(40),
        )),
    }];
    handlers.extend(
        roots
            .iter()
            .enumerate()
            .map(|(index, root)| CompactionHookHandler {
                id: Uuid::from_u128(index as u128 + 11),
                stage: CompactionHookStage::SessionStartCompact,
                hooks: Arc::new(hooks(
                    root.path(),
                    "cat >/dev/null; printf '%s' \"$$\" > pid; exec /bin/sleep 30",
                    1024,
                    Duration::from_secs(40),
                )),
            }),
    );
    let hooks = ConcurrentCompactionHooks::new(handlers, events.clone()).unwrap();
    let mut running = hooks.run_with_context(request(CompactionHookStage::SessionStartCompact));
    let ready = async {
        loop {
            let pids: Vec<_> = roots
                .iter()
                .filter_map(|root| {
                    std::fs::read_to_string(root.path().join("pid"))
                        .ok()?
                        .parse::<i32>()
                        .ok()
                })
                .collect();
            let completed = events.0.lock().unwrap().contains(&(
                Uuid::from_u128(10),
                CompactionHookNotification::Completed(CompactionHookOutcome::Continue),
            ));
            if pids.len() == 2 && completed {
                break pids;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    let pids = tokio::select! {
        _ = &mut running => panic!("pending commands must not complete"),
        pids = tokio::time::timeout(Duration::from_secs(2), ready) => pids.unwrap(),
    };
    assert_ne!(pids[0], pids[1]);
    for pid in &pids {
        assert_eq!(
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(*pid), None),
            Ok(())
        );
    }
    // The aggregate owns both process futures. No hook timeout is awaited.
    drop(running);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if pids.iter().all(|pid| {
                nix::sys::signal::kill(nix::unistd::Pid::from_raw(*pid), None)
                    == Err(nix::errno::Errno::ESRCH)
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let events = events.0.lock().unwrap();
    for (id, outcome) in [
        (10, CompactionHookOutcome::Continue),
        (11, CompactionHookOutcome::Canceled),
        (12, CompactionHookOutcome::Canceled),
    ] {
        assert_eq!(
            events
                .iter()
                .filter(|(actual, _)| *actual == Uuid::from_u128(id))
                .count(),
            2,
        );
        assert!(events.contains(&(Uuid::from_u128(id), CompactionHookNotification::Started)));
        assert!(events.contains(&(
            Uuid::from_u128(id),
            CompactionHookNotification::Completed(outcome),
        )));
    }
}
