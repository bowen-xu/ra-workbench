-- Rebuildable metadata only. Editable bodies, HBS state and save receipts remain
-- in their existing authoritative tables. Listing never opens note_blocks.
create table note_page_catalog (
    id text primary key references note_pages(id) on delete cascade,
    title text not null,
    category text not null check(category in ('independent','dependent')),
    source_id text, source_label text not null,
    updated_at text not null, status text not null,
    is_favorite integer not null, favorite_position integer not null,
    revision integer not null
);
create index note_page_catalog_recent on note_page_catalog(status, updated_at desc, id);
create index note_page_catalog_category_recent on note_page_catalog(status, category, updated_at desc, id);
create index note_page_catalog_favorites on note_page_catalog(status, is_favorite, favorite_position, id);
create table note_page_catalog_state(id integer primary key check(id=1), generation integer not null);
insert into note_page_catalog_state values(1,0);

-- Cache the reading title when the source/body metadata changes, rather than
-- opening each reading document while listing the library. Block order is DFS.
create view note_page_catalog_values as
with values_with_title as (
 select p.*, s.block_id as source_id, s.note_type, s.quote_exact, s.paper_title_snapshot,
    coalesce((
      with recursive blocks(id, path, text) as (
        select b.id, printf('%010d',b.position) || ':' || b.id, trim(b.plain_text,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))
        from note_blocks b where b.page_id=p.id and b.parent_block_id is null
        union all
        select b.id, t.path || '/' || printf('%010d',b.position) || ':' || b.id, trim(b.plain_text,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))
        from note_blocks b join blocks t on b.parent_block_id=t.id where b.page_id=p.id
      ) select trim(substr(text,1,instr(text || char(10),char(10))-1),' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))
        from blocks where text<>'' order by path limit 1
    ),'') as body_title
 from note_pages p left join reading_note_sources s on s.page_id=p.id
)
select p.id,
 case when source_id is null then title else substr(coalesce(nullif(body_title,''),
    nullif(trim(substr(ltrim(quote_exact,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)),1,instr(ltrim(quote_exact,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)) || char(10),char(10))-1),' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279)),''),title),1,100) end as title,
 case when note_type='highlight' or (note_type='annotation' and trim(quote_exact,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))<>'') then 'dependent' else 'independent' end as category,
 source_id,
 case when source_id is not null then paper_title_snapshot else coalesce((
   select group_concat(label,' · ') from (
     select json_extract(l.target_snapshot_json,'$.labelSnapshot') as label from note_links l
     where l.source_page_id=p.id and l.target_kind in ('paper','manuscript')
       and coalesce(json_extract(l.target_snapshot_json,'$.labelSnapshot'),'')<>'' order by l.created_at,l.id
   )
 ),'') end as source_label,
 updated_at,status,
 case when note_type='highlight' or (note_type='annotation' and trim(quote_exact,' ' || char(9,10,11,12,13,160,5760,8192,8193,8194,8195,8196,8197,8198,8199,8200,8201,8202,8232,8233,8239,8287,12288,65279))<>'') then 0 else is_favorite end as is_favorite,
 coalesce((select position from note_favorite_order f where f.page_id=p.id),9223372036854775807) as favorite_position,revision
from values_with_title p;

insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values;

create trigger note_page_catalog_generation_insert after insert on note_page_catalog
begin update note_page_catalog_state set generation=generation+1 where id=1; end;

create trigger note_page_catalog_generation_update after update on note_page_catalog
begin update note_page_catalog_state set generation=generation+1 where id=1; end;

create trigger note_page_catalog_generation_delete after delete on note_page_catalog
begin update note_page_catalog_state set generation=generation+1 where id=1; end;

create trigger note_page_catalog_note_pages_insert after insert on note_pages
begin
    delete from note_page_catalog where id = new.id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = new.id and 1;
end;

create trigger note_page_catalog_note_pages_update after update on note_pages
begin
    delete from note_page_catalog where id = new.id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = new.id and 1;
    delete from note_page_catalog where id = old.id and old.id<>new.id;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = old.id and old.id<>new.id;
end;

create trigger note_page_catalog_note_pages_delete after delete on note_pages
begin
    delete from note_page_catalog where id = old.id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = old.id and 1;
end;

create trigger note_page_catalog_reading_note_sources_insert after insert on reading_note_sources
begin
    delete from note_page_catalog where id = new.page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = new.page_id and 1;
end;

create trigger note_page_catalog_reading_note_sources_update after update on reading_note_sources
begin
    delete from note_page_catalog where id = new.page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = new.page_id and 1;
    delete from note_page_catalog where id = old.page_id and old.page_id<>new.page_id;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = old.page_id and old.page_id<>new.page_id;
end;

create trigger note_page_catalog_reading_note_sources_delete after delete on reading_note_sources
begin
    delete from note_page_catalog where id = old.page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = old.page_id and 1;
end;

create trigger note_page_catalog_note_links_insert after insert on note_links
begin
    delete from note_page_catalog where id = new.source_page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = new.source_page_id and 1;
end;

create trigger note_page_catalog_note_links_update after update on note_links
begin
    delete from note_page_catalog where id = new.source_page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = new.source_page_id and 1;
    delete from note_page_catalog where id = old.source_page_id and old.source_page_id<>new.source_page_id;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = old.source_page_id and old.source_page_id<>new.source_page_id;
end;

create trigger note_page_catalog_note_links_delete after delete on note_links
begin
    delete from note_page_catalog where id = old.source_page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = old.source_page_id and 1;
end;

create trigger note_page_catalog_note_favorite_order_insert after insert on note_favorite_order
begin
    delete from note_page_catalog where id = new.page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = new.page_id and 1;
end;

create trigger note_page_catalog_note_favorite_order_update after update on note_favorite_order
begin
    delete from note_page_catalog where id = new.page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = new.page_id and 1;
    delete from note_page_catalog where id = old.page_id and old.page_id<>new.page_id;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = old.page_id and old.page_id<>new.page_id;
end;

create trigger note_page_catalog_note_favorite_order_delete after delete on note_favorite_order
begin
    delete from note_page_catalog where id = old.page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = old.page_id and 1;
end;

create trigger note_page_catalog_blocks_insert after insert on note_blocks
when exists(select 1 from reading_note_sources s where s.page_id=new.page_id)
begin
    delete from note_page_catalog where id = new.page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = new.page_id and 1;
end;

create trigger note_page_catalog_blocks_update after update on note_blocks
when exists(select 1 from reading_note_sources s where s.page_id=new.page_id or s.page_id=old.page_id)
begin
    delete from note_page_catalog where id = new.page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = new.page_id and 1;
    delete from note_page_catalog where id = old.page_id and old.page_id<>new.page_id;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = old.page_id and old.page_id<>new.page_id;
end;

create trigger note_page_catalog_blocks_delete after delete on note_blocks
when exists(select 1 from reading_note_sources s where s.page_id=old.page_id)
begin
    delete from note_page_catalog where id = old.page_id and 1;
    insert into note_page_catalog(id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision) select id, title, category, source_id, source_label, updated_at, status, is_favorite, favorite_position, revision from note_page_catalog_values where id = old.page_id and 1;
end;
