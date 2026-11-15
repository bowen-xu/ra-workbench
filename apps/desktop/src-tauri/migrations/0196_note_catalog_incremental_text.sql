-- Rebuildable projection of committed JSON, independent of editable/HBS state.
-- The pure versioned function reuses the canonical validator and is registered
-- on every application connection before migration or note writes.
create view note_catalog_projection_values as
select b.id as block_id,b.page_id,b.parent_block_id as parent_id,b.position,result.value as projection
from note_blocks b cross join json_each(json_array(
 ra_note_block_projection_v1(b.block_type,b.props_json,b.content_json))) result
where b.scope_kind='page' and b.status='active';
create view note_catalog_block_values as
select block_id,page_id,parent_id,position,json_extract(projection,'$.text') as search_text,
 json_extract(projection,'$.firstLine') as first_line,json_extract(projection,'$.foldedText') as folded_text
from note_catalog_projection_values where json_extract(projection,'$.errorCode') is null;
create table note_catalog_projection_errors (
 block_id text primary key references note_blocks(id) on delete cascade,
 page_id text not null references note_pages(id) on delete cascade,
 error_code text not null,cause text not null
);
create view note_catalog_projection_error_values as
select block_id,page_id,json_extract(projection,'$.errorCode') as error_code,
 json_extract(projection,'$.cause') as cause
from note_catalog_projection_values where json_extract(projection,'$.errorCode') is not null;
insert into note_catalog_projection_errors select * from note_catalog_projection_error_values;
create trigger note_catalog_projection_error_insert after insert on note_blocks begin
 insert into note_catalog_projection_errors select * from note_catalog_projection_error_values where block_id=new.id; end;
create trigger note_catalog_projection_error_update after update of page_id,block_type,content_json,props_json,scope_kind,status on note_blocks
 when old.page_id is not new.page_id or old.block_type is not new.block_type or old.content_json is not new.content_json
 or old.props_json is not new.props_json or old.scope_kind is not new.scope_kind or old.status is not new.status
begin
 delete from note_catalog_projection_errors where block_id=old.id;
 insert into note_catalog_projection_errors select * from note_catalog_projection_error_values where block_id=new.id;
end;

create table note_catalog_blocks (
 block_id text primary key references note_blocks(id) on delete cascade,
 page_id text not null references note_pages(id) on delete cascade,
 parent_id text,position integer not null,search_text text not null,first_line text not null,folded_text text not null
);
create index note_catalog_blocks_tree on note_catalog_blocks(page_id,parent_id,position,block_id);
create table note_catalog_title_cache (
 page_id text primary key references note_pages(id) on delete cascade,body_title text not null
);
create table note_catalog_title_dirty (
 page_id text primary key references note_pages(id) on delete cascade
);

insert into note_catalog_blocks select * from note_catalog_block_values;

-- Walk short metadata only, and seek children by (page_id,parent_id).
create view note_catalog_title_values as
select p.id as page_id,coalesce((
 with recursive tree(block_id,path,first_line) as (
  select b.block_id,printf('%020d',b.position) || ':' || hex(b.block_id),b.first_line
  from note_catalog_blocks b where b.page_id=p.id and (b.parent_id is null or not exists(
   select 1 from note_catalog_blocks parent where parent.page_id=p.id and parent.block_id=b.parent_id))
  union all
  select b.block_id,t.path || '/' || printf('%020d',b.position) || ':' || hex(b.block_id),b.first_line
  from tree t cross join note_catalog_blocks b on b.parent_id=t.block_id where b.page_id=p.id
 ) select first_line from tree where first_line<>'' order by path limit 1
),'') as body_title from note_pages p;
insert into note_catalog_title_cache select * from note_catalog_title_values;

-- Keep only the page identity dirty while a canonical document transaction is
-- changing blocks. Metadata-only commits have nothing to refresh.
create trigger note_catalog_projection_dirty_insert after insert on note_catalog_blocks
begin insert into note_catalog_title_dirty select new.page_id where not exists(select 1 from note_catalog_title_dirty where page_id=new.page_id); end;
create trigger note_catalog_projection_dirty_delete after delete on note_catalog_blocks
when exists(select 1 from note_pages where id=old.page_id)
begin insert into note_catalog_title_dirty select old.page_id where not exists(select 1 from note_catalog_title_dirty where page_id=old.page_id); end;
create trigger note_catalog_projection_dirty_update after update on note_catalog_blocks
begin
 insert into note_catalog_title_dirty select new.page_id where not exists(select 1 from note_catalog_title_dirty where page_id=new.page_id);
 insert into note_catalog_title_dirty select old.page_id where old.page_id<>new.page_id and not exists(select 1 from note_catalog_title_dirty where page_id=old.page_id)
  and exists(select 1 from note_pages where id=old.page_id);
end;

create trigger note_catalog_projection_insert after insert on note_blocks
begin
 insert into note_catalog_blocks select * from note_catalog_block_values where block_id=new.id;
end;
create trigger note_catalog_projection_update
 after update of page_id,parent_block_id,position,block_type,content_json,props_json,scope_kind,status on note_blocks
 when old.page_id is not new.page_id or old.parent_block_id is not new.parent_block_id
  or old.position is not new.position or old.block_type is not new.block_type
  or old.content_json is not new.content_json or old.props_json is not new.props_json or old.scope_kind is not new.scope_kind or old.status is not new.status
begin
 delete from note_catalog_blocks where block_id=old.id and not exists(select 1 from note_catalog_block_values where block_id=new.id);
 insert into note_catalog_blocks select * from note_catalog_block_values where block_id=new.id
 on conflict(block_id) do update set page_id=excluded.page_id,parent_id=excluded.parent_id,
  position=excluded.position,search_text=excluded.search_text,first_line=excluded.first_line,folded_text=excluded.folded_text
 where page_id is not excluded.page_id or parent_id is not excluded.parent_id or position is not excluded.position
  or search_text is not excluded.search_text or first_line is not excluded.first_line or folded_text is not excluded.folded_text;
end;

-- Catalog metadata joins cached short text. It never traverses bodies when a
-- favorite, status, source label or other page metadata changes.
drop view note_page_catalog_values;
create view note_page_catalog_values as
select p.id,
 case when s.block_id is null then p.title else ra_note_display_title_v1(coalesce(c.body_title,''),s.quote_exact,p.title) end as title,
 case when s.note_type='highlight' or (s.note_type='annotation' and trim(s.quote_exact,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))<>'') then 'dependent' else 'independent' end as category,
 s.block_id as source_id,
 case when s.block_id is not null then s.paper_title_snapshot else coalesce((select group_concat(label,' · ') from (
  select json_extract(l.target_snapshot_json,'$.labelSnapshot') as label from note_links l
  where l.source_page_id=p.id and l.target_kind in ('paper','manuscript')
   and coalesce(json_extract(l.target_snapshot_json,'$.labelSnapshot'),'')<>'' order by l.created_at,l.id
 )),'') end as source_label,
 p.updated_at,p.status,
 case when s.note_type='highlight' or (s.note_type='annotation' and trim(s.quote_exact,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))<>'') then 0 else p.is_favorite end as is_favorite,
 coalesce(f.position,9223372036854775807) as favorite_position,p.revision
from note_pages p left join reading_note_sources s on s.page_id=p.id
 left join note_catalog_title_cache c on c.page_id=p.id left join note_favorite_order f on f.page_id=p.id;

drop trigger note_page_catalog_note_pages_update;
create trigger note_page_catalog_note_pages_update after update on note_pages
begin
 insert into note_catalog_title_cache select * from note_catalog_title_values
 where page_id=new.id and exists(select 1 from note_catalog_title_dirty where page_id=new.id)
 on conflict(page_id) do update set body_title=excluded.body_title;
 delete from note_catalog_title_dirty where page_id=new.id;
 delete from note_page_catalog where id=new.id;
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision from note_page_catalog_values where id=new.id;
 delete from note_page_catalog where id=old.id and old.id<>new.id;
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision from note_page_catalog_values where id=old.id and old.id<>new.id;
end;

drop trigger note_page_catalog_reading_note_sources_insert;
create trigger note_page_catalog_reading_note_sources_insert after insert on reading_note_sources
begin
 insert into note_catalog_title_cache select * from note_catalog_title_values
 where page_id=new.page_id and exists(select 1 from note_catalog_title_dirty where page_id=new.page_id)
 on conflict(page_id) do update set body_title=excluded.body_title;
 delete from note_catalog_title_dirty where page_id=new.page_id;
 delete from note_page_catalog where id=new.page_id;
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision from note_page_catalog_values where id=new.page_id;
end;

drop trigger note_page_catalog_reading_note_sources_update;
create trigger note_page_catalog_reading_note_sources_update after update on reading_note_sources
begin
 insert into note_catalog_title_cache select * from note_catalog_title_values
 where page_id=new.page_id and exists(select 1 from note_catalog_title_dirty where page_id=new.page_id)
 on conflict(page_id) do update set body_title=excluded.body_title;
 delete from note_catalog_title_dirty where page_id=new.page_id;
 delete from note_page_catalog where id=new.page_id;
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision from note_page_catalog_values where id=new.page_id;
 delete from note_page_catalog where id=old.page_id and old.page_id<>new.page_id;
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision from note_page_catalog_values where id=old.page_id and old.page_id<>new.page_id;
end;

-- Reconcile the derived headers without modifying canonical pages or revisions.
update note_page_catalog set title=(select v.title from note_page_catalog_values v where v.id=note_page_catalog.id);
