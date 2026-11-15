create table paper_context_snapshots (
    id text primary key,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    context_hash text not null unique check(length(context_hash) = 64),
    context_schema_version integer not null check(context_schema_version >= 1),
    created_at text not null
);

create table paper_context_attachments (
    snapshot_id text not null references paper_context_snapshots(id) on delete restrict,
    ordinal integer not null check(ordinal >= 0),
    attachment_id text not null references attachments(id) on delete restrict,
    content_sha256 text not null check(length(content_sha256) = 64),
    parse_run_id text not null references docling_parse_runs(id) on delete restrict,
    primary key(snapshot_id, ordinal),
    unique(snapshot_id, attachment_id)
);

create table paper_context_items (
    snapshot_id text not null references paper_context_snapshots(id) on delete restrict,
    context_item_id text not null check(length(context_item_id) between 1 and 80),
    attachment_id text not null references attachments(id) on delete restrict,
    content_sha256 text not null check(length(content_sha256) = 64),
    parse_run_id text not null references docling_parse_runs(id) on delete restrict,
    raw_ref text not null,
    item_kind text not null check(item_kind in (
        'text', 'table', 'picture', 'group', 'key_value', 'form', 'field', 'field_region'
    )),
    label text,
    page_index integer check(page_index is null or page_index >= 0),
    regions_json text not null check(json_valid(regions_json) and json_type(regions_json) = 'array'),
    charspan_json text check(
        charspan_json is null or (json_valid(charspan_json) and json_type(charspan_json) = 'array')
    ),
    source_text_hash text check(source_text_hash is null or length(source_text_hash) = 64),
    text_content text,
    table_json text check(table_json is null or json_valid(table_json)),
    image_content_sha256 text references content_objects(sha256) on delete restrict,
    created_at text not null,
    primary key(snapshot_id, context_item_id)
);

create index paper_context_snapshots_paper_idx
    on paper_context_snapshots(paper_id, created_at desc);

create index paper_context_items_parse_run_idx
    on paper_context_items(parse_run_id, context_item_id);

alter table ai_runs
    add column context_snapshot_id text references paper_context_snapshots(id) on delete restrict;

alter table paper_ai_reading_sessions
    add column context_snapshot_id text references paper_context_snapshots(id) on delete restrict;

drop view content_object_references;

create view content_object_references as
select content_sha256, 'attachment' as owner_kind, id as owner_id, 'content' as role
from attachments
union all
select content_sha256, 'source_anchor' as owner_kind, id as owner_id, 'source' as role
from source_anchors
union all
select content_sha256, 'note_block' as owner_kind, block_id as owner_id, 'live_note_asset' as role
from note_block_assets
union all
select content_sha256, 'note_revision' as owner_kind, revision_id as owner_id, 'history_asset' as role
from note_revision_assets
union all
select content_sha256, 'manuscript_compile_artifact' as owner_kind, id as owner_id, 'compile_artifact' as role
from manuscript_compile_artifacts
where content_sha256 is not null
union all
select content_sha256, 'docling_parse_run' as owner_kind, id as owner_id, 'structured_pdf_parse' as role
from docling_parse_runs
union all
select image_content_sha256, 'paper_ai_reading_turn_context' as owner_kind, id as owner_id, 'paper_image' as role
from paper_ai_reading_turn_context_items
where image_content_sha256 is not null
union all
select image_content_sha256, 'ai_docling_evidence_context' as owner_kind, evidence_item_id as owner_id, 'paper_image' as role
from ai_docling_evidence_context_items
where image_content_sha256 is not null
union all
select image_content_sha256, 'paper_context_snapshot' as owner_kind,
       snapshot_id || ':' || context_item_id as owner_id, 'paper_image' as role
from paper_context_items
where image_content_sha256 is not null;

create trigger content_object_gc_paper_context_item_insert
after insert on paper_context_items
when new.image_content_sha256 is not null
begin
    delete from content_object_gc_candidates
    where content_sha256 = new.image_content_sha256;
end;
