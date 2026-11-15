create table reference_records (
    id text primary key,
    citing_paper_id text not null references bibliographic_items(id) on delete cascade,
    source_anchor_id text not null references source_anchors(id) on delete restrict,
    raw_text text not null check(length(trim(raw_text)) between 1 and 8000),
    fields_json text not null default '[]' check(json_valid(fields_json)),
    parse_state text not null check(parse_state in ('unparsed', 'parsed', 'failed')),
    identity_state text not null check(identity_state in (
        'unverified', 'matched_existing', 'verified_external', 'ambiguous', 'not_found'
    )),
    matched_item_id text references bibliographic_items(id) on delete set null,
    semantic_state text not null check(semantic_state in ('unreviewed', 'reviewed')),
    usage_kind text check(usage_kind is null or usage_kind in (
        'mentions', 'uses_method', 'supports', 'opposes', 'extends', 'background'
    )),
    usage_summary text not null default '' check(length(usage_summary) <= 4000),
    usage_anchor_id text references source_anchors(id) on delete restrict,
    provenance_kind text not null check(provenance_kind in (
        'human_original', 'ai_accepted_unchanged', 'ai_accepted_after_edit'
    )),
    current_revision integer not null check(current_revision >= 1),
    created_at text not null,
    updated_at text not null,
    check(
        (identity_state = 'matched_existing' and matched_item_id is not null)
        or identity_state != 'matched_existing'
    ),
    check(
        (semantic_state = 'reviewed' and usage_kind is not null
         and length(trim(usage_summary)) > 0 and usage_anchor_id is not null)
        or
        (semantic_state = 'unreviewed' and usage_kind is null
         and length(usage_summary) = 0 and usage_anchor_id is null)
    )
);

create table reference_record_revisions (
    id text primary key,
    reference_record_id text not null references reference_records(id) on delete restrict,
    revision integer not null check(revision >= 1),
    snapshot_json text not null check(json_valid(snapshot_json)),
    editor_kind text not null check(editor_kind in ('human', 'ai_review')),
    change_reason text not null check(length(change_reason) between 1 and 120),
    created_at text not null,
    unique(reference_record_id, revision)
);

create index reference_records_paper_idx
    on reference_records(citing_paper_id, updated_at desc);
create index reference_records_identity_idx
    on reference_records(identity_state, matched_item_id);

create trigger reference_record_revisions_no_update
before update on reference_record_revisions begin
    select raise(abort, 'reference record revisions are immutable');
end;

create trigger reference_record_revisions_no_delete
before delete on reference_record_revisions begin
    select raise(abort, 'reference record revisions are immutable');
end;
