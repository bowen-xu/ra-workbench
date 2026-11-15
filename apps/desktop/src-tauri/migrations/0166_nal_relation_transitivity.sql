-- Explicit binary relation rules; inheritance never substitutes product arguments.
create table nal_relation_rules (
    relation_concept_id text primary key references concepts(id) on delete cascade,
    kind text not null check(kind = 'transitive')
);
alter table nal_state add column relation_semantics_version integer not null default 0
    check(relation_semantics_version in (0,1));
