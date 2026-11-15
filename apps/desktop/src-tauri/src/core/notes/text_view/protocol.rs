use serde::Deserialize;
use std::{collections::BTreeMap, sync::LazyLock};
#[derive(Deserialize)]
pub(super) struct Tag {
    pub attributes: Vec<String>,
}
#[derive(Deserialize)]
pub(super) struct Protocol {
    pub version: u32,
    pub tags: BTreeMap<String, Tag>,
    pub colors: Vec<String>,
    pub alignments: Vec<String>,
}
pub(super) static DEFINITION: LazyLock<Protocol> = LazyLock::new(|| {
    serde_json::from_str(include_str!("protocol.json"))
        .expect("compiled note text protocol must be valid")
});
