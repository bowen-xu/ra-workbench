use super::*;
use fixture::{PDF, text};
use rusqlite::{params, types::Value};

fn input(attachment: &str, view_state_json: Option<String>) -> SaveReadingStateInputV1 {
    SaveReadingStateInputV1 {
        item_id: "paper".into(),
        attachment_id: attachment.into(),
        page_index: -7,
        zoom: F64BitsV1::from_f64(9.0),
        view_state_json,
    }
}

fn save(f: &Fixture, request: SaveReadingStateInputV1, bounded: f64) {
    f.run(
        AttachmentRequest::SaveState {
            input: request.clone(),
        },
        Some(AttachmentPlanV1::SaveReadingState {
            request,
            bounded_zoom: F64BitsV1::from_f64(bounded),
        }),
    )
    .unwrap();
}

fn get(
    f: &Fixture,
    attachment: &str,
) -> Result<Option<research_workbench_native_sdk::attachments::ReadingPosition>, AttachmentFailure>
{
    let result = f.run(
        AttachmentRequest::GetState {
            item_id: "paper".into(),
            attachment_id: attachment.into(),
        },
        None,
    )?;
    let AttachmentResult::State(value) = result else {
        panic!("position result");
    };
    Ok(value)
}

#[test]
fn state_save_uses_native_prepared_zoom_original_page_sql_and_unchanged_optional_raw_json() {
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    assert!(get(&f, &attachment.id).unwrap().is_none());
    let raw = " { \"schemaVersion\":2, \"pageIndex\":3, \"scale\":{\"kind\":\"page-fit\"}, \"scrollMode\":\"vertical\", \"spreadMode\":\"none\" } ";
    save(&f, input(&attachment.id, Some(raw.into())), 3.0);
    let state = get(&f, &attachment.id).unwrap().unwrap();
    assert_eq!(state.page_index, 0);
    assert_eq!(state.zoom.to_bits(), 3.0_f64.to_bits());
    assert_eq!(state.view_state_json.as_deref(), Some(raw));
    assert_eq!(
        f.rows("select view_state_json from reading_states"),
        vec![vec![text(raw)]]
    );
    assert!(f.store.connection().unwrap().is_autocommit());
    save(&f, input(&attachment.id, None), 0.5);
    let state = get(&f, &attachment.id).unwrap().unwrap();
    assert_eq!(state.zoom.to_bits(), 0.5_f64.to_bits());
    assert!(state.view_state_json.is_none());
    assert_eq!(
        f.rows("select view_state_json from reading_states"),
        vec![vec![Value::Null]]
    );
    let prefix = f.prefix();
    assert_eq!(f.reopen().prefix(), prefix);
}

#[test]
fn original_read_f64_bits_nullable_json_and_i32_before_bad_zoom_are_preserved() {
    let f = Fixture::new();
    let attachment = f.attach("paper", "source.pdf", PDF);
    save(&f, input(&attachment.id, None), 1.0);
    for zoom in [f64::INFINITY, f64::from_bits(1), 1.0 + f64::EPSILON] {
        let original = {
            let db = f.store.connection().unwrap();
            db.execute(
                "update reading_states set page_index=2147483647,zoom=?1",
                [zoom],
            )
            .unwrap();
            db.query_row("select zoom from reading_states", [], |r| {
                r.get::<_, f64>(0)
            })
            .unwrap()
        };
        let state = get(&f, &attachment.id).unwrap().unwrap();
        assert_eq!(state.page_index, i32::MAX);
        assert_eq!(state.zoom.to_bits(), original.to_bits());
        assert!(state.view_state_json.is_none());
    }
    f.store
        .connection()
        .unwrap()
        .execute(
            "update reading_states set page_index=?1,zoom=?2",
            params![i64::from(i32::MAX) + 1, "private invalid zoom"],
        )
        .unwrap();
    let failure = get(&f, &attachment.id).unwrap_err();
    assert_eq!(failure.code, "storage_error");
    assert_eq!(
        failure.data.as_ref().unwrap().safe_cause,
        "integral_value_out_of_range"
    );
    assert_eq!(
        failure.data.as_ref().unwrap().resource_id,
        "literature.attachments.reading-state-get"
    );
    assert!(
        !serde_json::to_string(&failure)
            .unwrap()
            .contains("private invalid zoom")
    );
    f.store
        .connection()
        .unwrap()
        .execute("update reading_states set page_index=0", [])
        .unwrap();
    assert_eq!(
        get(&f, &attachment.id)
            .unwrap_err()
            .data
            .as_ref()
            .unwrap()
            .safe_cause,
        "invalid_column_type"
    );
    assert!(f.store.connection().unwrap().is_autocommit());
}
