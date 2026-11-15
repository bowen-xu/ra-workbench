create table ai_docling_evidence_context_items (
    evidence_item_id text primary key references ai_evidence_items(id) on delete cascade,
    context_item_id text not null check(length(context_item_id) between 1 and 80),
    parse_run_id text not null references docling_parse_runs(id) on delete restrict,
    raw_ref text not null,
    item_kind text not null,
    label text,
    regions_json text not null,
    charspan_json text,
    source_text_hash text,
    image_content_sha256 text references content_objects(sha256) on delete restrict,
    created_at text not null
);

create index ai_docling_evidence_context_run_idx
    on ai_docling_evidence_context_items(parse_run_id);
