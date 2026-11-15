-- Query definitions are durable; the derived materialisation can be rebuilt.
create table nal_query_collections (
    id integer primary key check (id = 1),
    revision integer not null default 0 check (revision >= 0),
    definitions text not null default '[]' check (json_valid(definitions)),
    cache text check (cache is null or json_valid(cache)),
    legacy_imported integer not null default 0 check (legacy_imported in (0, 1))
);
insert into nal_query_collections (id) values (1);
