//! Application-owned event delivery. Tauri and isolated preview transports share
//! event payloads; neither transport owns the pending operation or its ACK.
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;
use tauri::Emitter;

type Sender = dyn Fn(&'static str, Value) -> Result<(), String> + Send + Sync;

#[derive(Clone)]
pub(crate) struct RuntimeEvents(Arc<Sender>);

impl RuntimeEvents {
    pub(crate) fn new(
        send: impl Fn(&'static str, Value) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        Self(Arc::new(send))
    }

    pub(crate) fn for_app(app: tauri::AppHandle) -> Self {
        Self::new(move |name, payload| app.emit(name, payload).map_err(|error| error.to_string()))
    }

    pub(crate) fn emit(&self, name: &'static str, payload: impl Serialize) -> Result<(), String> {
        let payload =
            serde_json::to_value(payload).map_err(|_| "runtime_event_encode_failed".to_owned())?;
        (self.0)(name, payload)
    }
}
