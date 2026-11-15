create table bibliographic_items_next (
    id text primary key,
    item_type text not null check(item_type in (
        'journalArticle', 'magazineArticle', 'newspaperArticle', 'conferencePaper',
        'book', 'bookSection', 'preprint', 'manuscript', 'thesis', 'report',
        'dataset', 'computerProgram', 'webpage', 'standard', 'patent', 'document'
    )),
    title text not null default '',
    abstract_text text not null default '',
    issued_year integer,
    container_title text not null default '',
    volume text not null default '',
    issue text not null default '',
    pages text not null default '',
    url text not null default '',
    language text not null default '',
    bibtex_key text not null default '',
    raw_bibtex text,
    reading_status text not null default 'unread'
        check(reading_status in ('unread', 'reading', 'read')),
    archived_at text,
    created_at text not null,
    updated_at text not null,
    trashed_at text
);

insert into bibliographic_items_next (
    id, item_type, title, abstract_text, issued_year, container_title, volume,
    issue, pages, url, language, bibtex_key, raw_bibtex, reading_status,
    archived_at, created_at, updated_at, trashed_at
)
select
    id, item_type, title, abstract_text, issued_year, container_title, volume,
    issue, pages, url, language, bibtex_key, raw_bibtex, reading_status,
    archived_at, created_at, updated_at, trashed_at
from bibliographic_items;

drop table bibliographic_items;
alter table bibliographic_items_next rename to bibliographic_items;

create index bibliographic_items_created_at_idx on bibliographic_items(created_at desc);
create index bibliographic_items_trashed_at_idx on bibliographic_items(trashed_at);
