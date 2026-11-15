-- Rebuildable metadata keys only. Canonical pages, documents, save revisions
-- and HBS state are neither rewritten nor used as a second authority here.
alter table note_page_catalog add column catalog_title_key blob not null default X'';
alter table note_page_catalog add column catalog_title_down blob not null default X'';
alter table note_page_catalog add column catalog_category_key blob not null default X'';
alter table note_page_catalog add column catalog_category_down blob not null default X'';
alter table note_page_catalog add column catalog_source_key blob not null default X'';
alter table note_page_catalog add column catalog_source_down blob not null default X'';
alter table note_page_catalog add column catalog_updated_ms integer;

create table note_catalog_order_state (
    id integer primary key check(id=1),
    collation_version text not null,
    time_version text not null
);
insert into note_catalog_order_state values(1,'icu4x-2.3.1/zh-CN/kn-primary/v1','rfc3339-utc-ms-floor-v1');

-- MATERIALIZED computes each ICU key once, including the initial metadata-only
-- backfill. The paired reverse key preserves ascending stable-ID tie breaks.
update note_page_catalog set
 (catalog_title_key,catalog_title_down,catalog_category_key,catalog_category_down,
  catalog_source_key,catalog_source_down,catalog_updated_ms) = (
    with keys as materialized (
      select ra_note_sort_key_v1(note_page_catalog.title) as title_key,
        ra_note_sort_key_v1(case note_page_catalog.category when 'independent' then '独立笔记' else '附属笔记' end) as category_key,
        ra_note_sort_key_v1(note_page_catalog.source_label) as source_key
    ) select title_key,ra_note_reverse_key_v1(title_key),category_key,ra_note_reverse_key_v1(category_key),
      source_key,ra_note_reverse_key_v1(source_key),ra_note_utc_millis_v1(note_page_catalog.updated_at) from keys
 );

create trigger note_catalog_order_insert after insert on note_page_catalog
begin
    update note_page_catalog set
      (catalog_title_key,catalog_title_down,catalog_category_key,catalog_category_down,catalog_source_key,catalog_source_down,catalog_updated_ms) = (
        with keys as materialized (
          select ra_note_sort_key_v1(new.title) as title_key,
            ra_note_sort_key_v1(case new.category when 'independent' then '独立笔记' else '附属笔记' end) as category_key,
            ra_note_sort_key_v1(new.source_label) as source_key
        ) select title_key,ra_note_reverse_key_v1(title_key),category_key,ra_note_reverse_key_v1(category_key),
          source_key,ra_note_reverse_key_v1(source_key),ra_note_utc_millis_v1(new.updated_at) from keys
      ) where id=new.id;
end;

create trigger note_catalog_order_text_update after update of title,category,source_label on note_page_catalog
when old.title is not new.title or old.category is not new.category or old.source_label is not new.source_label
begin
    update note_page_catalog set
      (catalog_title_key,catalog_title_down,catalog_category_key,catalog_category_down,catalog_source_key,catalog_source_down) = (
        with keys as materialized (
          select ra_note_sort_key_v1(new.title) as title_key,
            ra_note_sort_key_v1(case new.category when 'independent' then '独立笔记' else '附属笔记' end) as category_key,
            ra_note_sort_key_v1(new.source_label) as source_key
        ) select title_key,ra_note_reverse_key_v1(title_key),category_key,ra_note_reverse_key_v1(category_key),
          source_key,ra_note_reverse_key_v1(source_key) from keys
      ) where id=new.id;
end;

create trigger note_catalog_order_time_update after update of updated_at on note_page_catalog
when old.updated_at is not new.updated_at
begin
    update note_page_catalog set catalog_updated_ms=ra_note_utc_millis_v1(new.updated_at) where id=new.id;
end;

create index note_catalog_invalid_time on note_page_catalog(status,category,id) where catalog_updated_ms is null;
create index note_catalog_recent_utc on note_page_catalog(status,catalog_updated_ms desc,id);
create index note_catalog_category_recent_utc on note_page_catalog(status,category,catalog_updated_ms desc,id);
create index note_catalog_time_up_all on note_page_catalog(status,catalog_updated_ms,id);
create index note_catalog_time_up_category on note_page_catalog(status,category,catalog_updated_ms,id);
create index note_catalog_time_up_favorite on note_page_catalog(status,is_favorite,catalog_updated_ms,id);
create index note_catalog_time_down_favorite on note_page_catalog(status,is_favorite,catalog_updated_ms desc,id);

create index note_catalog_title_up_all on note_page_catalog(status,catalog_title_key,id);
create index note_catalog_title_up_category on note_page_catalog(status,category,catalog_title_key,id);
create index note_catalog_title_up_favorite on note_page_catalog(status,is_favorite,catalog_title_key,id);
create index note_catalog_title_down_all on note_page_catalog(status,catalog_title_down,id);
create index note_catalog_title_down_category on note_page_catalog(status,category,catalog_title_down,id);
create index note_catalog_title_down_favorite on note_page_catalog(status,is_favorite,catalog_title_down,id);
create index note_catalog_category_up_all on note_page_catalog(status,catalog_category_key,id);
create index note_catalog_category_up_category on note_page_catalog(status,category,catalog_category_key,id);
create index note_catalog_category_up_favorite on note_page_catalog(status,is_favorite,catalog_category_key,id);
create index note_catalog_category_down_all on note_page_catalog(status,catalog_category_down,id);
create index note_catalog_category_down_category on note_page_catalog(status,category,catalog_category_down,id);
create index note_catalog_category_down_favorite on note_page_catalog(status,is_favorite,catalog_category_down,id);
create index note_catalog_source_up_all on note_page_catalog(status,catalog_source_key,id);
create index note_catalog_source_up_category on note_page_catalog(status,category,catalog_source_key,id);
create index note_catalog_source_up_favorite on note_page_catalog(status,is_favorite,catalog_source_key,id);
create index note_catalog_source_down_all on note_page_catalog(status,catalog_source_down,id);
create index note_catalog_source_down_category on note_page_catalog(status,category,catalog_source_down,id);
create index note_catalog_source_down_favorite on note_page_catalog(status,is_favorite,catalog_source_down,id);
