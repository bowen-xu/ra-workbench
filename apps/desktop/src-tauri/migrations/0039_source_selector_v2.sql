alter table source_anchors
add column selector_json text
check (
    selector_json is null
    or (
        json_valid(selector_json)
        and json_extract(selector_json, '$.schemaVersion') = 2
        and json_type(selector_json, '$.segments') = 'array'
        and json_array_length(selector_json, '$.segments') between 1 and 2
    )
);

alter table reading_note_sources
add column selector_json text
check (
    selector_json is null
    or (
        json_valid(selector_json)
        and json_extract(selector_json, '$.schemaVersion') = 2
        and json_type(selector_json, '$.segments') = 'array'
        and json_array_length(selector_json, '$.segments') between 1 and 2
    )
);
