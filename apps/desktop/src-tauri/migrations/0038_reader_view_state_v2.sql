alter table reading_states
add column view_state_json text
check (
    view_state_json is null
    or (
        json_valid(view_state_json)
        and json_extract(view_state_json, '$.schemaVersion') = 2
        and json_type(view_state_json, '$.pageIndex') = 'integer'
        and json_extract(view_state_json, '$.pageIndex') >= 0
    )
);
