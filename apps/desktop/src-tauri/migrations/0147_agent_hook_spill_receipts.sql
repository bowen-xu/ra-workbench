-- Immutable native output receipts; the model's locator text grants nothing.
create table agent_hook_spill_artifacts (
 id text primary key not null check(length(id)=36),
 source_conversation_id text not null check(length(source_conversation_id)=36),
 source_window_id text not null check(length(source_window_id)=36),
 turn_id text not null check(length(turn_id)=36),
 phase text not null check(phase in ('manual','pre_turn','mid_turn')),
 model_json text not null check(json_valid(model_json) and json_type(model_json)='object'),
 content_sha256 text not null references content_objects(sha256),
 created_at text not null default(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
create index agent_hook_spill_artifacts_source on agent_hook_spill_artifacts(source_conversation_id,source_window_id,turn_id);
create trigger agent_hook_spill_artifacts_no_update before update on agent_hook_spill_artifacts
begin select raise(abort,'hook spill artifacts are immutable'); end;
create table agent_hook_spill_previews (
 artifact_id text primary key not null references agent_hook_spill_artifacts(id) on delete cascade,
 preview_sha256 text not null check(length(preview_sha256)=64 and preview_sha256 not glob '*[^0-9a-f]*')
);
create trigger agent_hook_spill_previews_no_update before update on agent_hook_spill_previews
begin select raise(abort,'hook spill previews are immutable'); end;
create table agent_hook_spill_grants (
 conversation_id text not null references agent_conversations(id) on delete cascade,
 window_id text not null references agent_context_checkpoints(id) on delete cascade,
 source_audit_seq integer not null check(source_audit_seq>0),
 artifact_id text not null references agent_hook_spill_previews(artifact_id),
 turn_id text not null,
 boundary_seq integer not null check(boundary_seq>0),
 boundary_ordinal integer generated always as (boundary_seq-1) stored,
 primary key(conversation_id,window_id,source_audit_seq,artifact_id),
 foreign key(conversation_id,boundary_ordinal) references agent_items(conversation_id,ordinal) on delete cascade
);
create index agent_hook_spill_grants_artifact on agent_hook_spill_grants(artifact_id);
create trigger agent_hook_spill_grants_source before insert on agent_hook_spill_grants
begin
 select case when not exists (
  select 1 from agent_compact_hook_contexts c where c.conversation_id=new.conversation_id
   and c.window_id=new.window_id and c.audit_seq=new.source_audit_seq
   and c.turn_id=new.turn_id and c.boundary_seq=new.boundary_seq
  union all
  select 1 from agent_inherited_hook_contexts c where c.conversation_id=new.conversation_id
   and c.window_id=new.window_id and c.source_audit_seq=new.source_audit_seq
   and c.turn_id=new.turn_id and c.boundary_seq=new.boundary_seq
 ) then raise(abort,'spill grant lacks completed hook context') end;
end;
create trigger agent_hook_spill_grants_no_update before update on agent_hook_spill_grants
begin select raise(abort,'hook spill grants are immutable'); end;
-- A fork keeps its own grant after origin deletion. Drop orphan evidence only
-- when both the origin and every granted descendant have been removed.
create trigger agent_hook_spill_origin_delete after delete on agent_conversations
begin
 delete from agent_hook_spill_artifacts where source_conversation_id=old.id
  and not exists(select 1 from agent_hook_spill_grants g where g.artifact_id=agent_hook_spill_artifacts.id);
end;
create trigger agent_hook_spill_grant_delete after delete on agent_hook_spill_grants
begin
 delete from agent_hook_spill_artifacts where id=old.artifact_id
  and not exists(select 1 from agent_conversations c where c.id=source_conversation_id)
  and not exists(select 1 from agent_hook_spill_grants g where g.artifact_id=old.artifact_id);
end;

-- Preserve the current reference view, including reading source retention.
drop view content_object_references;
create view content_object_references as
select content_sha256, 'attachment' as owner_kind, id as owner_id, 'content' as role from attachments
union all select content_sha256,'source_anchor',id,'source' from source_anchors
union all select content_sha256,'note_block',block_id,'live_note_asset' from note_block_assets
union all select content_sha256,'note_revision',revision_id,'history_asset' from note_revision_assets
union all select content_sha256,'manuscript_compile_artifact',id,'compile_artifact' from manuscript_compile_artifacts where content_sha256 is not null
union all select content_sha256,'docling_parse_run',id,'structured_pdf_parse' from docling_parse_runs
union all select image_content_sha256,'paper_ai_reading_turn_context',id,'paper_image' from paper_ai_reading_turn_context_items where image_content_sha256 is not null
union all select image_content_sha256,'ai_docling_evidence_context',evidence_item_id,'paper_image' from ai_docling_evidence_context_items where image_content_sha256 is not null
union all select image_content_sha256,'paper_context_snapshot',snapshot_id||':'||context_item_id,'paper_image' from paper_context_items where image_content_sha256 is not null
union all select before_sha256,'agent_change_set',change_set_id,'before' from agent_change_set_files where before_sha256 is not null
union all select after_sha256,'agent_change_set',change_set_id,'after' from agent_change_set_files where after_sha256 is not null
union all select content_sha256,'reading_note_source',page_id,'source' from reading_note_sources
union all select content_sha256,'agent_hook_spill_artifact',id,'history_asset' from agent_hook_spill_artifacts;
create trigger content_object_gc_hook_spill_insert after insert on agent_hook_spill_artifacts
begin delete from content_object_gc_candidates where content_sha256=new.content_sha256; end;
create trigger content_object_gc_hook_spill_delete after delete on agent_hook_spill_artifacts
when exists(select 1 from content_objects where sha256=old.content_sha256)
 and not exists(select 1 from content_object_references where content_sha256=old.content_sha256)
begin
 insert into content_object_gc_candidates(content_sha256,unreferenced_since,not_before,reason)
 values(old.content_sha256,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now','+30 days'),'live_reference_removed')
 on conflict(content_sha256) do nothing;
end;
