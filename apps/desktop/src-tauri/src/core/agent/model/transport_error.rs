use research_agent_core::model::ModelError;

/// Keep the failure class, never reqwest's URL/body/credential-bearing display.
pub(super) fn request(error: reqwest::Error) -> ModelError {
    if error.is_timeout() {
        ModelError::TimedOut
    } else if error.is_connect() {
        ModelError::ConnectionFailed
    } else {
        ModelError::Unavailable
    }
}

pub(super) fn response(error: reqwest::Error) -> ModelError {
    if error.is_timeout() {
        ModelError::TimedOut
    } else {
        ModelError::ResponseReadFailed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn distinguishes_connect_and_timeout_without_exposing_request_details() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let error = reqwest::Client::new()
            .get(format!("http://{address}/fixture"))
            .send()
            .await
            .unwrap_err();
        assert_eq!(request(error), ModelError::ConnectionFailed);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let error = reqwest::Client::new()
            .get(format!("http://{address}/fixture"))
            .timeout(Duration::from_millis(200))
            .send()
            .await
            .unwrap_err();
        assert_eq!(request(error), ModelError::TimedOut);
        task.abort();
    }

    #[tokio::test]
    async fn identifies_response_connection_failure() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\nshort")
                .await
                .unwrap();
        });
        let mut reply = reqwest::get(format!("http://{address}/fixture"))
            .await
            .unwrap();
        loop {
            match reply.chunk().await {
                Ok(Some(_)) => continue,
                Err(error) => {
                    assert_eq!(response(error), ModelError::ResponseReadFailed);
                    break;
                }
                Ok(None) => panic!("truncated response must fail"),
            }
        }
        task.await.unwrap();
    }
}
