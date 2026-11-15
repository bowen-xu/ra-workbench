pub use research_workbench_native_sdk::note_catalog_query::{PageCatalogQuery, PageCatalogSort};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageCatalogPage {
    pub items: Vec<PageCatalogItem>,
    pub revision: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageCatalogItem {
    pub id: String,
    pub title: String,
    pub snippet: String,
    pub category: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<PageCatalogOwner>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_label: Option<String>,
    pub updated_at: String,
    pub status: String,
    pub is_favorite: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub favorite_position: Option<i64>,
    pub target: PageCatalogTarget,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageCatalogOwner {
    pub kind: &'static str,
    pub id: String,
    pub label: String,
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PageCatalogTarget {
    Page { page_id: String },
    Reading { page_id: String, note_id: String },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct PageCatalogCursor {
    pub schema: u8,
    pub revision: i64,
    pub category: String,
    pub view: String,
    pub updated_at: String,
    pub id: String,
    pub favorite_position: i64,
}
