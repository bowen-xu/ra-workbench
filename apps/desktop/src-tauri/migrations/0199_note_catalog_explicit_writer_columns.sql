-- Preserve 0193-0198 exactly. Their original catalog had ten columns;
-- explicitly name those columns now that derived ordering columns exist.
-- Conflict predicates, source/title refresh, canonical data and HBS are unchanged.

drop trigger note_page_catalog_note_pages_insert;
create trigger note_page_catalog_note_pages_insert after insert on note_pages
begin
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=new.id and 1
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
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=new.id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=old.id and old.id<>new.id
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
end;

drop trigger note_page_catalog_note_pages_delete;
create trigger note_page_catalog_note_pages_delete after delete on note_pages
begin
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=old.id and 1
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
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=new.page_id and 1
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
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=new.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=old.page_id and old.page_id<>new.page_id
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
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=old.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=old.page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

drop trigger note_page_catalog_note_links_insert;
create trigger note_page_catalog_note_links_insert after insert on note_links
begin
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=new.source_page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=new.source_page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

drop trigger note_page_catalog_note_links_update;
create trigger note_page_catalog_note_links_update after update on note_links
begin
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=new.source_page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=old.source_page_id and old.source_page_id<>new.source_page_id
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
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=old.source_page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_catalog_metadata_text select * from note_catalog_metadata_text_values where page_id=old.source_page_id
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

drop trigger note_page_catalog_note_favorite_order_insert;
create trigger note_page_catalog_note_favorite_order_insert after insert on note_favorite_order
begin
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=new.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
end;

drop trigger note_page_catalog_note_favorite_order_update;
create trigger note_page_catalog_note_favorite_order_update after update on note_favorite_order
begin
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=new.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=old.page_id and old.page_id<>new.page_id
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
end;

drop trigger note_page_catalog_note_favorite_order_delete;
create trigger note_page_catalog_note_favorite_order_delete after delete on note_favorite_order
begin
 insert into note_page_catalog(id,title,category,source_id,source_label,updated_at,status,is_favorite,favorite_position,revision) select * from note_page_catalog_values where id=old.page_id and 1
 on conflict(id) do update set title=excluded.title,category=excluded.category,source_id=excluded.source_id,source_label=excluded.source_label,updated_at=excluded.updated_at,status=excluded.status,is_favorite=excluded.is_favorite,favorite_position=excluded.favorite_position,revision=excluded.revision
 where note_page_catalog.title is not excluded.title or note_page_catalog.category is not excluded.category or note_page_catalog.source_id is not excluded.source_id or note_page_catalog.source_label is not excluded.source_label or note_page_catalog.updated_at is not excluded.updated_at or note_page_catalog.status is not excluded.status or note_page_catalog.is_favorite is not excluded.is_favorite or note_page_catalog.favorite_position is not excluded.favorite_position or note_page_catalog.revision is not excluded.revision;
end;
