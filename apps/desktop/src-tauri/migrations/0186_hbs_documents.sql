-- HBS stores only the additional review baseline for notes. The authoritative
-- document remains in note_blocks; structured resources keep their own B/S.
create table hbs_note_baselines (
    page_id text primary key references note_pages(id) on delete cascade,
    baseline_json text not null check(json_valid(baseline_json))
);
create table hbs_commit_receipts (
    kind text not null,
    resource_id text not null,
    request_id text not null,
    fingerprint text not null,
    result_json text not null check(json_valid(result_json)),
    primary key(kind, resource_id, request_id)
);
create table hbs_text_documents (
    id text primary key,
    revision integer not null default 0 check(revision >= 0),
    content text not null,
    baseline text not null
);
