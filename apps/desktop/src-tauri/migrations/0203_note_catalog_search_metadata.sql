-- Extend the existing derived metadata index; never read canonical blocks or
-- change authoring/HBS revisions. Earlier migration bytes remain unchanged.
create index note_catalog_tags_reverse on note_page_tags(tag_id,page_id);
create index note_catalog_projects_reverse on project_note_pages(page_id,project_id);
create index note_catalog_creators_reverse on item_creators(creator_id,item_id);
create index note_catalog_attachment_reverse on reading_note_sources(attachment_id_snapshot,page_id);

drop view note_catalog_metadata_text_values;
create view note_catalog_metadata_text_values as
select page_id,search_text,ra_note_fold_v1(search_text) as folded_text from (
 select c.id as page_id,
 c.title || ' ' || c.source_label || ' ' || coalesce(s.quote_exact,'') || ' ' || coalesce((
  select target_id from note_links where source_page_id=c.id and source_block_id is null and target_kind='paper'
  and relation_type='derived_from' order by created_at,id limit 1),'') || ' ' || coalesce((
  select group_concat(t.name,' ') from note_page_tags x join tags t on t.id=x.tag_id where x.page_id=c.id),'') || ' ' || coalesce((
  select group_concat(r.title,' ') from project_note_pages x join research_projects r on r.id=x.project_id where x.page_id=c.id),'') || ' ' || coalesce((
  select group_concat(case when a.literal_name<>'' then a.literal_name else trim(a.family_name || ' ' || a.given_name) end,' ')
  from item_creators x join creators a on a.id=x.creator_id where x.item_id=s.paper_id_snapshot),'') || ' ' ||
 case when s.block_id is null then '' when p.id is null then 'unavailable' when p.trashed_at is not null then 'paper_in_trash'
  when s.current_anchor_id is null then 'page_only' when a.content_sha256<>s.content_sha256 then 'needs_relocation'
  when s.note_type='page_note' then 'page_only' else 'exact' end as search_text
 from note_page_catalog c left join reading_note_sources s on s.page_id=c.id
 left join bibliographic_items p on p.id=s.paper_id_snapshot left join attachments a on a.id=s.attachment_id_snapshot
);

-- Register old identities only. Search fills these in bounded transactions;
-- startup/list readiness never waits for extended metadata text or FTS work.
create table note_catalog_metadata_pending (
 page_id text primary key references note_page_catalog(id) on delete cascade
);
insert into note_catalog_metadata_pending(page_id) select id from note_page_catalog;
create trigger note_catalog_metadata_pending_insert after insert on note_catalog_metadata_text
begin delete from note_catalog_metadata_pending where page_id=new.page_id; end;
create trigger note_catalog_metadata_pending_update after update of search_text on note_catalog_metadata_text
begin delete from note_catalog_metadata_pending where page_id=new.page_id; end;

create trigger note_catalog_metadata_note_page_tags_insert after insert on note_page_tags
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(new.page_id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_note_page_tags_update after update on note_page_tags
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(new.page_id,old.page_id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_note_page_tags_delete after delete on note_page_tags
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(old.page_id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_project_note_pages_insert after insert on project_note_pages
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(new.page_id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_project_note_pages_update after update on project_note_pages
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(new.page_id,old.page_id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_project_note_pages_delete after delete on project_note_pages
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(old.page_id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_tag_name after update of name on tags
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from note_page_tags where tag_id in(new.id,old.id))
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_project_title after update of title on research_projects
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from project_note_pages where project_id in(new.id,old.id))
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_item_creators_insert after insert on item_creators
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from reading_note_sources where paper_id_snapshot in(new.item_id))
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_item_creators_update after update on item_creators
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from reading_note_sources where paper_id_snapshot in(new.item_id,old.item_id))
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_item_creators_delete after delete on item_creators
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from reading_note_sources where paper_id_snapshot in(old.item_id))
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_creator_name after update of literal_name,family_name,given_name on creators
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select s.page_id from reading_note_sources s join item_creators x on x.item_id=s.paper_id_snapshot where x.creator_id in(new.id,old.id))
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_bibliographic_items_insert after insert on bibliographic_items
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from reading_note_sources where paper_id_snapshot=new.id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_bibliographic_items_update after update of trashed_at on bibliographic_items
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from reading_note_sources where paper_id_snapshot=new.id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_bibliographic_items_delete after delete on bibliographic_items
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from reading_note_sources where paper_id_snapshot=old.id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_attachments_insert after insert on attachments
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from reading_note_sources where attachment_id_snapshot=new.id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_attachments_update after update of content_sha256 on attachments
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from reading_note_sources where attachment_id_snapshot=new.id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;

create trigger note_catalog_metadata_attachments_delete after delete on attachments
begin
 insert into note_catalog_metadata_text(page_id,search_text,folded_text)
 select page_id,search_text,folded_text from note_catalog_metadata_text_values where page_id in(select page_id from reading_note_sources where attachment_id_snapshot=old.id)
 on conflict(page_id) do update set search_text=excluded.search_text,folded_text=excluded.folded_text where search_text is not excluded.search_text;
end;
