-- Reading annotations own ordinary note pages. Their legacy IDs remain source
-- aliases only; no independently writable annotation body survives this upgrade.
create table reading_note_page_map (
    legacy_id text primary key,
    page_id text not null unique references note_pages(id) on delete cascade
);
-- A separate page identity prevents collisions with existing page IDs.
insert into reading_note_page_map(legacy_id, page_id)
select id, lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4' ||
       substr(lower(hex(randomblob(2))),2) || '-a' || substr(lower(hex(randomblob(2))),2) || '-' || lower(hex(randomblob(6)))
from note_blocks where scope_kind = 'reading';
insert into note_pages(id,title,position,status,is_favorite,revision,schema_version,created_at,updated_at,trashed_at)
select m.page_id, substr(case when s.paper_title_snapshot='' then '阅读笔记' else s.paper_title_snapshot end,1,240),
       (select coalesce(max(position),-1)+1 from note_pages)+row_number() over(order by b.id)-1,
       b.status,b.is_favorite,b.revision,1,b.created_at,b.updated_at,b.trashed_at
from note_blocks b join reading_note_page_map m on m.legacy_id=b.id
join reading_note_sources s on s.block_id=b.id;
insert into note_page_tags(page_id,tag_id)
select m.page_id,t.tag_id from reading_note_tags t join reading_note_page_map m on m.legacy_id=t.block_id;
insert into project_note_pages(page_id,project_id,added_at)
select m.page_id,t.project_id,t.added_at from project_reading_notes t join reading_note_page_map m on m.legacy_id=t.block_id;

-- Split literal newlines without interpreting Markdown or changing source quotes.
with recursive lines(page_id,part,remaining,position,created_at,updated_at) as (
 select m.page_id,'',coalesce(json_extract(b.content_json,'$.body'),json_extract(b.content_json,'$.items[0].text'),'') || char(10),-1,b.created_at,b.updated_at
 from note_blocks b join reading_note_page_map m on m.legacy_id=b.id
 where coalesce(json_extract(b.content_json,'$.body'),json_extract(b.content_json,'$.items[0].text'),'')<>''
 union all
 select page_id,substr(remaining,1,instr(remaining,char(10))-1),substr(remaining,instr(remaining,char(10))+1),position+1,created_at,updated_at
 from lines where remaining<>''
)
insert into note_blocks(id,scope_kind,page_id,position,block_type,props_json,content_json,plain_text,schema_version,revision,status,is_favorite,created_at,updated_at)
select lower(hex(randomblob(16))),'page',page_id,position,'paragraph','{}',
       json_object('kind','inline','items',json(case when part='' then '[]' else json_array(json_object('type','text','text',part,'styles',json_array())) end)),part,1,0,'active',0,created_at,updated_at
from lines where position>=0;

create table reading_note_sources_next (
 block_id text primary key,
 page_id text not null unique references note_pages(id) on delete cascade,
 note_type text not null check(note_type in ('highlight','annotation','page_note')),
 color text not null,
 current_anchor_id text references source_anchors(id) on delete set null,
 paper_id_snapshot text not null, paper_title_snapshot text not null,
 attachment_id_snapshot text not null, attachment_name_snapshot text not null,
 content_sha256 text not null, page_index integer not null check(page_index>=0),
 page_label text, rects_json text not null check(json_valid(rects_json)),
 quote_exact text not null, quote_prefix text not null, quote_suffix text not null,
 locator_version integer not null, created_at text not null, updated_at text not null,
 selector_json text
);
insert into reading_note_sources_next
select s.block_id,m.page_id,case b.block_type when 'reading_highlight' then 'highlight' when 'reading_page_note' then 'page_note' else 'annotation' end,
       coalesce(json_extract(b.props_json,'$.color'),'#f4cf65'),s.current_anchor_id,s.paper_id_snapshot,s.paper_title_snapshot,
       s.attachment_id_snapshot,s.attachment_name_snapshot,s.content_sha256,s.page_index,s.page_label,s.rects_json,s.quote_exact,
       s.quote_prefix,s.quote_suffix,s.locator_version,s.created_at,s.updated_at,s.selector_json
from reading_note_sources s join reading_note_page_map m on m.legacy_id=s.block_id join note_blocks b on b.id=s.block_id;
drop table reading_note_sources;
alter table reading_note_sources_next rename to reading_note_sources;
create index reading_note_sources_paper_page_idx on reading_note_sources(paper_id_snapshot,page_index);
create index reading_note_sources_current_anchor_idx on reading_note_sources(current_anchor_id);
delete from note_blocks where scope_kind='reading';
drop table reading_note_tags;
drop table project_reading_notes;
-- Read-only aliases retain compatibility with old references, not old writers.
create view reading_note_tags as select s.block_id,t.tag_id from reading_note_sources s join note_page_tags t on t.page_id=s.page_id;
create view project_reading_notes as select s.block_id,t.project_id,t.added_at from reading_note_sources s join project_note_pages t on t.page_id=s.page_id;
update note_index_state set state='dirty' where id=1;

-- Keep retained source attachments reachable independently of an active anchor.
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
where image_content_sha256 is not null
union all
select before_sha256, 'agent_change_set' as owner_kind, change_set_id as owner_id, 'before' as role
from agent_change_set_files where before_sha256 is not null
union all
select after_sha256, 'agent_change_set' as owner_kind, change_set_id as owner_id, 'after' as role
from agent_change_set_files where after_sha256 is not null
union all
select content_sha256, 'reading_note_source', page_id, 'source' from reading_note_sources;
create trigger content_object_gc_reading_source_insert after insert on reading_note_sources
begin
 delete from content_object_gc_candidates where content_sha256=new.content_sha256;
end;
create trigger content_object_gc_reading_source_delete after delete on reading_note_sources
when exists(select 1 from content_objects where sha256=old.content_sha256)
 and not exists(select 1 from content_object_references where content_sha256=old.content_sha256)
begin
 insert into content_object_gc_candidates(content_sha256,unreferenced_since,not_before,reason)
 values(old.content_sha256,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now','+30 days'),'live_reference_removed')
 on conflict(content_sha256) do nothing;
end;
delete from content_object_gc_candidates where content_sha256 in(select content_sha256 from reading_note_sources);
