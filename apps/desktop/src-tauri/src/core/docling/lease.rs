use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::{net::TcpStream, sync::watch};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::HeaderValue},
};

use super::{
    error::DoclingError,
    protocol::{LeaseGranted, LeaseHeartbeat, LeaseHeartbeatAck, LeaseHello, PROTOCOL_VERSION},
};

pub(crate) type LeaseSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(crate) async fn open_lease(
    websocket_url: &str,
    token: &str,
    client_id: &str,
    expected_instance_id: &str,
) -> Result<(LeaseSocket, LeaseGranted), DoclingError> {
    let mut request = websocket_url
        .into_client_request()
        .map_err(|error| DoclingError::Request(error.to_string()))?;
    request.headers_mut().insert(
        "X-Api-Key",
        HeaderValue::from_str(token).map_err(|error| DoclingError::Request(error.to_string()))?,
    );
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), connect_async(request))
        .await
        .map_err(|_| DoclingError::Request("连接 Docling heartbeat 超时".to_owned()))?
        .map_err(|error| DoclingError::Request(error.to_string()))?;
    let hello = serde_json::to_string(&LeaseHello {
        r#type: "hello",
        protocol_version: PROTOCOL_VERSION,
        client_id,
    })?;
    socket.send(Message::Text(hello.into())).await?;
    let granted = receive_lease(&mut socket).await?;
    if granted.r#type != "lease" || granted.service_instance_id != expected_instance_id {
        return Err(DoclingError::InvalidResponse(
            "Docling heartbeat 返回了不匹配的实例".to_owned(),
        ));
    }
    Ok((socket, granted))
}

pub(crate) async fn run_lease_loop<F>(
    mut socket: LeaseSocket,
    lease_id: &str,
    heartbeat_interval_ms: u64,
    lease_timeout_ms: u64,
    mut stop: watch::Receiver<bool>,
    mut update_counts: F,
) -> Option<String>
where
    F: FnMut(Option<u32>, Option<u32>),
{
    let heartbeat_interval = Duration::from_millis(heartbeat_interval_ms);
    let lease_timeout = Duration::from_millis(lease_timeout_ms);
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + heartbeat_interval,
        heartbeat_interval,
    );
    let mut last_ack = tokio::time::Instant::now();
    loop {
        tokio::select! {
            _ = interval.tick() => {
                if last_ack.elapsed() >= lease_timeout {
                    return Some("Docling heartbeat 已超时".to_owned());
                }
                let heartbeat = serde_json::to_string(&LeaseHeartbeat {
                    r#type: "heartbeat",
                    lease_id,
                }).ok()?;
                if let Err(error) = socket.send(Message::Text(heartbeat.into())).await {
                    return Some(error.to_string());
                }
            }
            changed = stop.changed() => {
                if changed.is_ok() && *stop.borrow() {
                    if let Ok(release) = serde_json::to_string(&json!({"type": "release"})) {
                        let _ = socket.send(Message::Text(release.into())).await;
                    }
                    let _ = socket.close(None).await;
                    return None;
                }
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(ack) = serde_json::from_str::<LeaseHeartbeatAck>(&text)
                            && ack.r#type == "heartbeatAck"
                            && ack.lease_id == lease_id
                        {
                            last_ack = tokio::time::Instant::now();
                            update_counts(ack.active_client_count, ack.active_request_count);
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if let Err(error) = socket.send(Message::Pong(payload)).await {
                            return Some(error.to_string());
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        return Some("Docling heartbeat 连接已关闭".to_owned());
                    }
                    Some(Err(error)) => return Some(error.to_string()),
                    _ => {}
                }
            }
        }
    }
}

async fn receive_lease(socket: &mut LeaseSocket) -> Result<LeaseGranted, DoclingError> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let message = tokio::time::timeout_at(deadline, socket.next())
            .await
            .map_err(|_| DoclingError::Request("等待 Docling lease 超时".to_owned()))?
            .ok_or_else(|| DoclingError::Request("Docling lease socket 已关闭".to_owned()))??;
        match message {
            Message::Text(text) => {
                return serde_json::from_str(&text)
                    .map_err(|error| DoclingError::InvalidResponse(error.to_string()));
            }
            Message::Ping(payload) => socket.send(Message::Pong(payload)).await?,
            Message::Close(_) => {
                return Err(DoclingError::Request(
                    "Docling lease socket 在授予租约前关闭".to_owned(),
                ));
            }
            Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}
