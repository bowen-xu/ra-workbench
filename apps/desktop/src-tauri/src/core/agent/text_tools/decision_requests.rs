//! One execution owner per user decision; definitive failures stay definitive.
use super::*;
use decisions::{Decision, Receipt};
use rusqlite::{OptionalExtension, params};
#[derive(Serialize)]
pub struct Failure {
    pub status: &'static str,
    pub message: String,
}
fn unknown(message: impl ToString) -> Failure {
    Failure {
        status: "unknown",
        message: message.to_string(),
    }
}
fn failed(message: impl ToString) -> Failure {
    Failure {
        status: "failed",
        message: message.to_string(),
    }
}
fn alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        if pid == 0 || pid > i32::MAX as u32 {
            return false;
        }
        // A different living process retains ownership; PID reuse is conservative.
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None)
            != Err(nix::errno::Errno::ESRCH)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        true
    }
}
pub(super) async fn run(store: AppStore, request: Decision) -> Result<Receipt, Failure> {
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&request).map_err(unknown)?)
    );
    {
        let mut connection = store.connection().map_err(unknown)?;
        let db = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(unknown)?;
        if let Some(receipt) = decisions::existing(&db, &request).map_err(unknown)? {
            return Ok(receipt);
        }
        let row:Option<(String,String,u32,String,Option<String>)>=db.query_row("select input_sha256,owner_boot,owner_pid,state,error from agent_note_decision_requests where request_id=?1",[request.request_id.to_string()],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?))).optional().map_err(unknown)?;
        if let Some((hash, boot, pid, state, error)) = row {
            if hash != fingerprint {
                return Err(failed("request identity belongs to another decision"));
            }
            if state == "failed" {
                return Err(failed(error.unwrap_or("original decision failed".into())));
            }
            if state == "complete" || boot == store.text_patch_boot().to_string() || alive(pid) {
                return Err(unknown(
                    "original decision is still running or requires result recovery",
                ));
            }
            db.execute("update agent_note_decision_requests set owner_boot=?2,owner_pid=?3 where request_id=?1",params![request.request_id.to_string(),store.text_patch_boot().to_string(),std::process::id()]).map_err(unknown)?;
        } else {
            db.execute("insert into agent_note_decision_requests(request_id,conversation_id,input_sha256,owner_boot,owner_pid,state) values(?1,?2,?3,?4,?5,'running')",params![request.request_id.to_string(),request.conversation_id.to_string(),fingerprint,store.text_patch_boot().to_string(),std::process::id()]).map_err(unknown)?;
        }
        db.commit().map_err(unknown)?;
    }
    let result = decisions::execute(store.clone(), request.clone()).await;
    let db = store.connection().map_err(unknown)?;
    // Receipt and page mutations share one transaction. A readable absent receipt
    // confirms rollback; a failed result query retains the unknown owner.
    let receipt = decisions::existing(&db, &request).map_err(unknown)?;
    if let Some(receipt) = receipt {
        db.execute("update agent_note_decision_requests set state='complete',error=null where request_id=?1",[request.request_id.to_string()]).map_err(unknown)?;
        return Ok(receipt);
    }
    let error = result
        .err()
        .unwrap_or("decision completion did not retain its receipt".into());
    db.execute(
        "update agent_note_decision_requests set state='failed',error=?2 where request_id=?1",
        params![request.request_id.to_string(), error],
    )
    .map_err(unknown)?;
    Err(failed(error))
}
