-- Persistent, incrementally maintained search indexes over derived text only.
create table note_catalog_search_state (id integer primary key check(id=1),generation integer not null);
insert into note_catalog_search_state(id,generation)values(1,0);
create virtual table note_catalog_block_search using fts5(folded_text,
 content='note_catalog_blocks',content_rowid='rowid',tokenize='trigram');
insert into note_catalog_block_search(note_catalog_block_search)values('rebuild');
create trigger note_catalog_block_search_insert after insert on note_catalog_blocks begin
 insert into note_catalog_block_search(rowid,folded_text)values(new.rowid,new.folded_text);
 update note_catalog_search_state set generation=generation+1 where id=1; end;
create trigger note_catalog_block_search_delete after delete on note_catalog_blocks begin
 insert into note_catalog_block_search(note_catalog_block_search,rowid,folded_text)values('delete',old.rowid,old.folded_text);
 update note_catalog_search_state set generation=generation+1 where id=1; end;
create trigger note_catalog_block_search_update after update of folded_text on note_catalog_blocks
 when old.folded_text is not new.folded_text begin
 insert into note_catalog_block_search(note_catalog_block_search,rowid,folded_text)values('delete',old.rowid,old.folded_text);
 insert into note_catalog_block_search(rowid,folded_text)values(new.rowid,new.folded_text); end;

create table note_catalog_metadata_text (
 page_id text primary key references note_page_catalog(id) on delete cascade,search_text text not null,folded_text text not null
);
create view note_catalog_metadata_text_values as
select page_id,search_text,ra_note_fold_v1(search_text) as folded_text from (select c.id as page_id,c.title || ' ' || c.source_label || ' ' || coalesce(s.quote_exact,'') || ' ' || coalesce((
 select target_id from note_links where source_page_id=c.id and source_block_id is null and target_kind='paper'
 and relation_type='derived_from' order by created_at,id limit 1),'') as search_text
from note_page_catalog c left join reading_note_sources s on s.page_id=c.id);
insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values;
create virtual table note_catalog_metadata_search using fts5(folded_text,
 content='note_catalog_metadata_text',content_rowid='rowid',tokenize='trigram');
insert into note_catalog_metadata_search(note_catalog_metadata_search)values('rebuild');
create trigger note_catalog_metadata_search_insert after insert on note_catalog_metadata_text begin
 insert into note_catalog_metadata_search(rowid,folded_text)values(new.rowid,new.folded_text);
 update note_catalog_search_state set generation=generation+1 where id=1; end;
create trigger note_catalog_metadata_search_delete after delete on note_catalog_metadata_text begin
 insert into note_catalog_metadata_search(note_catalog_metadata_search,rowid,folded_text)values('delete',old.rowid,old.folded_text);
 update note_catalog_search_state set generation=generation+1 where id=1; end;
create trigger note_catalog_metadata_search_update after update of folded_text on note_catalog_metadata_text
 when old.folded_text is not new.folded_text begin
 insert into note_catalog_metadata_search(note_catalog_metadata_search,rowid,folded_text)values('delete',old.rowid,old.folded_text);
 insert into note_catalog_metadata_search(rowid,folded_text)values(new.rowid,new.folded_text); end;
create trigger note_catalog_metadata_projection_insert after insert on note_page_catalog begin
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=new.id; end;
create trigger note_catalog_metadata_projection_update after update on note_page_catalog begin
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=new.id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text; end;

drop trigger note_page_catalog_note_pages_insert;
create trigger note_page_catalog_note_pages_insert after insert on note_pages
begin
 insert into note_page_catalog select * from note_page_catalog_values where id=new.id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
end;

drop trigger note_page_catalog_note_pages_update;
create trigger note_page_catalog_note_pages_update after update on note_pages
begin
 insert into note_catalog_title_cache select * from note_catalog_title_values
 where page_id=new.id and exists(select 1 from note_catalog_title_dirty where page_id=new.id)
 on conflict(page_id) do update set body_title=excluded.body_title;
 delete from note_catalog_title_dirty where page_id=new.id;
 insert into note_page_catalog select * from note_page_catalog_values where id=new.id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_page_catalog select * from note_page_catalog_values where id=old.id and old.id<>new.id
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
end;

drop trigger note_page_catalog_note_pages_delete;
create trigger note_page_catalog_note_pages_delete after delete on note_pages
begin
 insert into note_page_catalog select * from note_page_catalog_values where id=old.id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 delete from note_page_catalog where id=old.id;
end;

drop trigger note_page_catalog_reading_note_sources_insert;
create trigger note_page_catalog_reading_note_sources_insert after insert on reading_note_sources
begin
 insert into note_catalog_title_cache select * from note_catalog_title_values
 where page_id=new.page_id and exists(select 1 from note_catalog_title_dirty where page_id=new.page_id)
 on conflict(page_id) do update set body_title=excluded.body_title;
 delete from note_catalog_title_dirty where page_id=new.page_id;
 insert into note_page_catalog select * from note_page_catalog_values where id=new.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=new.page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

drop trigger note_page_catalog_reading_note_sources_update;
create trigger note_page_catalog_reading_note_sources_update after update on reading_note_sources
begin
 insert into note_catalog_title_cache select * from note_catalog_title_values
 where page_id=new.page_id and exists(select 1 from note_catalog_title_dirty where page_id=new.page_id)
 on conflict(page_id) do update set body_title=excluded.body_title;
 delete from note_catalog_title_dirty where page_id=new.page_id;
 insert into note_page_catalog select * from note_page_catalog_values where id=new.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_page_catalog select * from note_page_catalog_values where id=old.page_id and old.page_id<>new.page_id
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=new.page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=old.page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

drop trigger note_page_catalog_reading_note_sources_delete;
create trigger note_page_catalog_reading_note_sources_delete after delete on reading_note_sources
begin
 insert into note_page_catalog select * from note_page_catalog_values where id=old.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=old.page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

drop trigger note_page_catalog_note_links_insert;
create trigger note_page_catalog_note_links_insert after insert on note_links
begin
 insert into note_page_catalog select * from note_page_catalog_values where id=new.source_page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=new.source_page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

drop trigger note_page_catalog_note_links_update;
create trigger note_page_catalog_note_links_update after update on note_links
begin
 insert into note_page_catalog select * from note_page_catalog_values where id=new.source_page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_page_catalog select * from note_page_catalog_values where id=old.source_page_id and old.source_page_id<>new.source_page_id
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=new.source_page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=old.source_page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

drop trigger note_page_catalog_note_links_delete;
create trigger note_page_catalog_note_links_delete after delete on note_links
begin
 insert into note_page_catalog select * from note_page_catalog_values where id=old.source_page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=old.source_page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

drop trigger note_page_catalog_note_favorite_order_insert;
create trigger note_page_catalog_note_favorite_order_insert after insert on note_favorite_order
begin
 insert into note_page_catalog select * from note_page_catalog_values where id=new.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
end;

drop trigger note_page_catalog_note_favorite_order_update;
create trigger note_page_catalog_note_favorite_order_update after update on note_favorite_order
begin
 insert into note_page_catalog select * from note_page_catalog_values where id=new.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_page_catalog select * from note_page_catalog_values where id=old.page_id and old.page_id<>new.page_id
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
end;

drop trigger note_page_catalog_note_favorite_order_delete;
create trigger note_page_catalog_note_favorite_order_delete after delete on note_favorite_order
begin
 insert into note_page_catalog select * from note_page_catalog_values where id=old.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
end;

update note_page_catalog set title=(select title from note_page_catalog_values where id=note_page_catalog.id);


-- Raw-text changes also invalidate cursors when folding leaves FTS unchanged.
create trigger note_catalog_block_search_generation_update after update of search_text on note_catalog_blocks
 when old.search_text is not new.search_text begin
 update note_catalog_search_state set generation=generation+1 where id=1; end;

-- Raw-text changes also invalidate cursors when folding leaves FTS unchanged.
create trigger note_catalog_metadata_search_generation_update after update of search_text on note_catalog_metadata_text
 when old.search_text is not new.search_text begin
 update note_catalog_search_state set generation=generation+1 where id=1; end;
