alter table note_revisions
    add column is_pinned integer not null default 0
    check (is_pinned in (0, 1));

create table note_revision_assets (
    revision_id text not null references note_revisions(id) on delete cascade,
    content_sha256 text not null references content_objects(sha256) on delete restrict,
    primary key (revision_id, content_sha256)
);

create index note_revision_assets_content_idx
    on note_revision_assets(content_sha256);

create table content_object_gc_candidates (
    content_sha256 text primary key references content_objects(sha256) on delete cascade,
    unreferenced_since text not null,
    not_before text not null,
    reason text not null check(reason in (
        'never_attached',
        'live_reference_removed',
        'revision_pruned',
        'reconciliation'
    )),
    attempt_count integer not null default 0 check(attempt_count >= 0),
    last_attempt_at text,
    last_error_code text
);

create index content_object_gc_due_idx
    on content_object_gc_candidates(not_before, content_sha256);

create table content_gc_state (
    id integer primary key check(id = 1),
    backfill_status text not null check(backfill_status in ('pending', 'running', 'complete', 'blocked')),
    backfill_cursor text,
    job_state text not null check(job_state in ('idle', 'running', 'interrupted', 'failed')),
    last_mark_at text,
    last_sweep_at text,
    last_success_count integer not null default 0 check(last_success_count >= 0),
    last_success_bytes integer not null default 0 check(last_success_bytes >= 0),
    last_error_code text,
    updated_at text not null
);

insert into content_gc_state (
    id,
    backfill_status,
    job_state,
    updated_at
) values (
    1,
    'pending',
    'idle',
    strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
);

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
where content_sha256 is not null;

create trigger content_object_gc_after_insert
after insert on content_objects
begin
    insert into content_object_gc_candidates (
        content_sha256,
        unreferenced_since,
        not_before,
        reason
    ) values (
        new.sha256,
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
        strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+7 days'),
        'never_attached'
    ) on conflict(content_sha256) do nothing;
end;

create trigger content_object_gc_attachment_insert
after insert on attachments
begin
    delete from content_object_gc_candidates where content_sha256 = new.content_sha256;
end;

create trigger content_object_gc_attachment_delete
after delete on attachments
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

create trigger content_object_gc_source_anchor_insert
after insert on source_anchors
begin
    delete from content_object_gc_candidates where content_sha256 = new.content_sha256;
end;

create trigger content_object_gc_source_anchor_delete
after delete on source_anchors
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

create trigger content_object_gc_note_block_asset_insert
after insert on note_block_assets
begin
    delete from content_object_gc_candidates where content_sha256 = new.content_sha256;
end;

create trigger content_object_gc_note_block_asset_delete
after delete on note_block_assets
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

create trigger content_object_gc_note_revision_asset_insert
after insert on note_revision_assets
begin
    delete from content_object_gc_candidates where content_sha256 = new.content_sha256;
end;

create trigger content_object_gc_note_revision_asset_delete
after delete on note_revision_assets
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
        'revision_pruned'
    ) on conflict(content_sha256) do update set
        reason = 'revision_pruned',
        unreferenced_since = excluded.unreferenced_since,
        not_before = excluded.not_before,
        attempt_count = 0,
        last_attempt_at = null,
        last_error_code = null;
end;

create trigger content_object_gc_compile_artifact_insert
after insert on manuscript_compile_artifacts
when new.content_sha256 is not null
begin
    delete from content_object_gc_candidates where content_sha256 = new.content_sha256;
end;

create trigger content_object_gc_compile_artifact_delete
after delete on manuscript_compile_artifacts
when old.content_sha256 is not null and not exists (
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
