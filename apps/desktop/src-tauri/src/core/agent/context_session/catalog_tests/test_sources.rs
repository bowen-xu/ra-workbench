//! Finite test reads over the caller's snapshot and canonical Native SQL/rules.
use crate::agent::{
    context_catalog::{ContextCatalogEntries, ContextCatalogError, ContextCatalogSource},
    paper_metadata::PaperMetadataError,
};
use rusqlite::Connection;

fn pairs(db: &Connection, sql: &str, key: &str) -> Result<Vec<(String, String)>, rusqlite::Error> {
    db.prepare(sql)?
        .query_map([key], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect()
}

pub(super) fn catalog() -> ContextCatalogSource {
    ContextCatalogSource {
        search_papers: Some(|db, query| {
            papers(
                db,
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../../tests/fixtures/native-resources-v1/literature/context-catalog-papers.v1.sql"
                )),
                query,
            )
        }),
        list_pdf_attachments: Some(|db, paper| {
            papers(
                db,
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../../tests/fixtures/native-resources-v1/literature/context-catalog-pdf-attachments.v1.sql"
                )),
                paper,
            )
        }),
        search_manuscripts: None,
    }
}

fn papers(
    db: &Connection,
    sql: &str,
    key: &str,
) -> Result<ContextCatalogEntries, ContextCatalogError> {
    pairs(db, sql, key).map_err(|_| ContextCatalogError::Unavailable)
}

pub(super) fn metadata(db: &Connection, paper: &str) -> Result<String, PaperMetadataError> {
    let header: String = db
        .query_row(
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../tests/fixtures/native-resources-v1/literature/context-metadata-header.v1.sql"
            )),
            [paper],
            |row| row.get(0),
        )
        .map_err(|_| PaperMetadataError::SourceUnavailable)?;
    let mut base: serde_json::Value =
        serde_json::from_str(&header).map_err(|_| PaperMetadataError::Unavailable)?;
    let creators = pairs(
        db,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/fixtures/native-resources-v1/literature/context-metadata-creators.v1.sql"
        )),
        paper,
    )
    .map_err(|_| PaperMetadataError::Unavailable)?;
    let identifiers = pairs(
        db,
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/fixtures/native-resources-v1/literature/context-metadata-identifiers.v1.sql"
        )),
        paper,
    )
    .map_err(|_| PaperMetadataError::Unavailable)?;
    // The fixture supplies the accepted response over the caller's observed rows.
    base["creators"] = serde_json::json!(creators);
    base["identifiers"] = serde_json::json!(identifiers);
    Ok(base.to_string())
}
