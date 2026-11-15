//! A responsive task may take arbitrarily long, including while its page count is unchanged.
use super::*;
use futures_util::{StreamExt, stream};
use std::future::Future;
use tokio::time::{Instant, interval, sleep_until};

pub(super) async fn for_response<R, P, PF, C, CF>(
    request: R,
    poll: P,
    mut cancel: C,
    control: &ParseControl,
    timeout_ms: u64,
) -> Result<Value, DoclingError>
where
    R: Future<Output = Result<Value, DoclingError>>,
    P: FnMut() -> PF,
    PF: Future<Output = Result<ParseProgress, DoclingError>>,
    C: FnMut() -> CF,
    CF: Future<Output = Result<(), DoclingError>>,
{
    // Polling is a separate future so a slow or stuck poll cannot block cancellation,
    // a finished conversion, or the inactivity deadline.
    let responses = stream::unfold(
        (interval(Duration::from_millis(300)), poll),
        |(mut tick, mut poll)| async move {
            tick.tick().await;
            Some((poll().await, (tick, poll)))
        },
    );
    tokio::pin!(request, responses);
    let timeout = Duration::from_millis(timeout_ms);
    let mut deadline = Instant::now() + timeout;
    let mut last_error = "尚未收到有效任务状态响应".to_owned();
    let mut cancel_sent = false;
    loop {
        tokio::select! {
            result = &mut request => return result,
            _ = control.cancelled(), if !cancel_sent => {
                control.stage("cancelling");
                cancel().await?;
                cancel_sent = true;
                deadline = Instant::now() + timeout;
            }
            Some(response) = responses.next() => match response {
                Ok(progress) if progress.is_valid() => {
                    deadline = Instant::now() + timeout;
                    last_error = "已收到有效任务状态响应，之后持续无响应".to_owned();
                    control.update(progress);
                }
                Ok(_) => last_error = "服务返回了无效的任务阶段或页数".to_owned(),
                Err(error) => last_error = error.to_string(),
            },
            _ = sleep_until(deadline) => return Err(DoclingError::ResponseTimeout {
                timeout_ms,
                request_id: control.id.clone(),
                last_error,
            }),
        }
    }
}

#[cfg(test)]
#[path = "wait/tests.rs"]
mod tests;
