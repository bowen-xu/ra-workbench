create table collection_state (
    id integer primary key check(id = 1),
    revision integer not null default 0 check(revision >= 0)
);
insert into collection_state (id, revision) values (1, 0);
create table collections (
    id text primary key,
    name text not null check(length(trim(name)) between 1 and 200),
    description text not null default '' check(length(description) <= 4000),
    concept_id text references concepts(id) on delete set null
);
create table collection_relation_types (
    id text primary key,
    name text not null check(length(trim(name)) between 1 and 120),
    inverse_name text not null check(length(trim(inverse_name)) between 1 and 120),
    directed integer not null check(directed in (0, 1)),
    built_in integer not null check(built_in in (0, 1))
);
insert into collection_relation_types (id, name, inverse_name, directed, built_in) values
    ('sub-field', 'sub-field', '包含子领域', 1, 1),
    ('related-field', 'related-field', 'related-field', 0, 1);
create table collection_relations (
    id text primary key,
    source_id text not null references collections(id) on delete cascade,
    target_id text not null references collections(id) on delete cascade,
    type_id text not null references collection_relation_types(id) on delete restrict,
    check(source_id <> target_id),
    unique(source_id, target_id, type_id)
);
create index collection_relations_target_idx on collection_relations(target_id);
create index collection_relations_type_idx on collection_relations(type_id);
create table collection_memberships (
    item_id text not null references bibliographic_items(id) on delete cascade,
    collection_id text not null references collections(id) on delete cascade,
    primary key(item_id, collection_id)
);
create index collection_memberships_collection_idx on collection_memberships(collection_id, item_id);
create table item_relation_assignments (
    item_id text not null references bibliographic_items(id) on delete cascade,
    relation_id text not null references collection_relations(id) on delete cascade,
    primary key(item_id, relation_id)
);
create index item_relation_assignments_relation_idx on item_relation_assignments(relation_id, item_id);
