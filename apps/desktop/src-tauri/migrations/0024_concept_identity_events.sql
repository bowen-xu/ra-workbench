create table concept_identity_events (
    id text primary key,
    operation text not null check(operation in ('merge', 'split')),
    survivor_concept_id text not null references concepts(id) on delete restrict,
    source_concept_ids_json text not null check(
        json_valid(source_concept_ids_json)
        and json_type(source_concept_ids_json) = 'array'
        and json_array_length(source_concept_ids_json) between 1 and 10
    ),
    snapshot_json text not null check(json_valid(snapshot_json)),
    rationale text not null default '' check(length(rationale) <= 2000),
    created_at text not null
);

create table concept_identity_mappings (
    source_concept_id text primary key references concepts(id) on delete restrict,
    canonical_concept_id text not null references concepts(id) on delete restrict,
    merge_event_id text not null references concept_identity_events(id) on delete restrict,
    created_at text not null,
    check(source_concept_id != canonical_concept_id)
);

create table concept_identity_relation_impacts (
    event_id text not null references concept_identity_events(id) on delete restrict,
    relation_assertion_id text not null references relation_assertions(id) on delete restrict,
    endpoint_role text not null check(endpoint_role in ('subject', 'object')),
    original_concept_id text not null,
    resolved_concept_id text not null,
    primary key(event_id, relation_assertion_id, endpoint_role)
);

create trigger concept_identity_events_no_update
before update on concept_identity_events begin
    select raise(abort, 'concept identity events are immutable');
end;
create trigger concept_identity_events_no_delete
before delete on concept_identity_events begin
    select raise(abort, 'concept identity events are immutable');
end;
create trigger concept_identity_relation_impacts_no_update
before update on concept_identity_relation_impacts begin
    select raise(abort, 'concept identity relation impacts are immutable');
end;
create trigger concept_identity_relation_impacts_no_delete
before delete on concept_identity_relation_impacts begin
    select raise(abort, 'concept identity relation impacts are immutable');
end;
create trigger concept_identity_mappings_no_update
before update on concept_identity_mappings begin
    select raise(abort, 'concept identity mappings must be replaced through a new event');
end;

create index concept_identity_events_survivor_idx
    on concept_identity_events(survivor_concept_id, created_at desc);
create index concept_identity_mappings_canonical_idx
    on concept_identity_mappings(canonical_concept_id, created_at desc);
create index concept_identity_relation_impacts_relation_idx
    on concept_identity_relation_impacts(relation_assertion_id, event_id);
