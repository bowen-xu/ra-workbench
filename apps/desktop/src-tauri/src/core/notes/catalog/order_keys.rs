//! Pure metadata key functions. No document reads, clock, I/O or authority.
use super::{NotesError, model::PageCatalogQuery};
use rusqlite::{Connection, functions::FunctionFlags};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

pub(super) const TIME_VERSION: &str = "rfc3339-utc-ms-floor-v1";

pub(super) fn utc_millis(text: &str) -> Option<i64> {
    // JS Date.parse rejects leap seconds. Keep canonical strings unchanged,
    // but do not silently normalize an unrepresentable display timestamp.
    if text.as_bytes().get(17..19) == Some(b"60") {
        return None;
    }
    let parsed = OffsetDateTime::parse(text, &Rfc3339).ok()?;
    i64::try_from(parsed.unix_timestamp_nanos().div_euclid(1_000_000)).ok()
}

pub(super) fn register(connection: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    connection.create_scalar_function("ra_note_sort_key_v1", 1, flags, |context| {
        research_note_query::note_sort_key(&context.get::<String>(0)?).map_err(|cause| {
            rusqlite::Error::UserFunctionError(Box::new(std::io::Error::other(cause)))
        })
    })?;
    connection.create_scalar_function("ra_note_reverse_key_v1", 1, flags, |context| {
        Ok(research_note_query::note_reverse_key(
            &context.get::<Vec<u8>>(0)?,
        ))
    })?;
    connection.create_scalar_function("ra_note_utc_millis_v1", 1, flags, |context| {
        Ok(utc_millis(&context.get::<String>(0)?))
    })
}

pub(super) fn check_ready(
    connection: &Connection,
    query: &PageCatalogQuery,
    searching: bool,
) -> Result<(), NotesError> {
    let (collation, time): (String, String) = connection.query_row(
        "select collation_version,time_version from note_catalog_order_state where id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    if collation != research_note_query::note_collation_version() || time != TIME_VERSION {
        return Err(NotesError::IndexFailed(
            "note_catalog_order_version：笔记排序投影版本不匹配，请重建派生目录。".to_owned(),
        ));
    }
    let status = if query.view == "archive" {
        "archived"
    } else if query.view == "trash" {
        "trashed"
    } else {
        "active"
    };
    let scope = if searching {
        "case when ?1='trash' then status='trashed' else status<>'trashed' end"
    } else {
        "status=?1 and (?2='all' or category=?2) and (?3=0 or is_favorite=1)"
    };
    let sql = format!(
        "select id,updated_at from note_page_catalog where catalog_updated_ms is null and ({scope}) limit 1"
    );
    let mut statement = connection.prepare(&sql)?;
    statement.raw_bind_parameter(
        1,
        if searching {
            query.view.as_str()
        } else {
            status
        },
    )?;
    if !searching {
        statement.raw_bind_parameter(2, &query.category)?;
        statement.raw_bind_parameter(3, i64::from(query.view == "favorites"))?;
    }
    let mut rows = statement.raw_query();
    if let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let text: String = row.get(1)?;
        let cause = OffsetDateTime::parse(&text, &Rfc3339).err().map_or_else(
            || "时间包含不可表示的闰秒".to_owned(),
            |error| error.to_string(),
        );
        return Err(NotesError::IndexFailed(format!(
            "note_catalog_timestamp_invalid：笔记编辑时间无效（pageId={id}）；{cause}。原始数据已保留。"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utc_milliseconds_match_browser_fixtures() {
        let samples: serde_json::Value = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../tests/contracts/note-catalog-time.json"
        )))
        .unwrap();
        for sample in samples.as_array().unwrap() {
            assert_eq!(
                utc_millis(sample["input"].as_str().unwrap()),
                sample["milliseconds"].as_i64(),
                "{}",
                sample["input"]
            );
        }
    }
}
