alter table identifiers rename to identifiers_with_unique_value;

create table identifiers (
    id text primary key,
    item_id text not null references bibliographic_items(id) on delete cascade,
    kind text not null,
    normalized_value text not null,
    display_value text not null
);

insert into identifiers (id, item_id, kind, normalized_value, display_value)
select id, item_id, kind, normalized_value, display_value
from identifiers_with_unique_value;

drop table identifiers_with_unique_value;

create index identifiers_item_idx on identifiers(item_id);
create index identifiers_normalized_idx on identifiers(kind, normalized_value);
