create table docling_settings (
    id integer primary key check(id = 1),
    enabled integer not null default 1 check(enabled in (0, 1)),
    executable text not null default 'docling-serve' check(length(executable) between 1 and 500),
    default_port integer not null default 5001 check(default_port between 1 and 65535),
    fallback_start_port integer not null default 5010 check(fallback_start_port between 1 and 65535),
    fallback_end_port integer not null default 5099 check(fallback_end_port between 1 and 65535),
    idle_shutdown_ms integer not null default 1200000 check(idle_shutdown_ms between 60000 and 86400000),
    request_timeout_ms integer not null default 300000 check(request_timeout_ms between 10000 and 3600000),
    updated_at text not null
);

insert into docling_settings (id, updated_at)
values (1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));

create table docling_parse_runs (
    id text primary key,
    item_id text not null references bibliographic_items(id) on delete cascade,
    attachment_id text not null references attachments(id) on delete cascade,
    content_sha256 text not null references content_objects(sha256) on delete restrict,
    service_base_url text not null,
    docling_status text not null,
    parse_status text not null check(parse_status in ('running', 'success', 'partial_success', 'failure')),
    json_content text check(json_content is null or json_valid(json_content)),
    raw_response text not null check(json_valid(raw_response)),
    error_message text,
    processing_time_seconds real,
    created_at text not null,
    finished_at text not null
);

create index docling_parse_runs_attachment_created_idx
    on docling_parse_runs(attachment_id, created_at desc);

create index docling_parse_runs_item_created_idx
    on docling_parse_runs(item_id, created_at desc);

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
from docling_parse_runs;

create trigger content_object_gc_docling_parse_run_insert
after insert on docling_parse_runs
begin
    delete from content_object_gc_candidates where content_sha256 = new.content_sha256;
end;

create trigger content_object_gc_docling_parse_run_delete
after delete on docling_parse_runs
when not exists (
    select 1 from content_object_references where content_sha256 = old.content_sha256
)
begin
    insert into content_object_gc_candidates (
        content_sha256, unreferenced_since, not_before, reason
    ) values (
        old.content_sha256,
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+30 days'),
        'live_reference_removed'
    ) on conflict(content_sha256) do nothing;
end;
