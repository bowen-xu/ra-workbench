create table ai_overview_profile_runs (
    run_id text primary key references ai_runs(id) on delete restrict,
    paper_id text not null,
    selection_revision integer not null check(selection_revision >= 1),
    profile_refs_json text not null check(
        json_valid(profile_refs_json)
        and json_type(profile_refs_json) = 'array'
        and json_array_length(profile_refs_json) between 1 and 3
    ),
    field_kinds_json text not null check(
        json_valid(field_kinds_json)
        and json_type(field_kinds_json) = 'array'
        and json_array_length(field_kinds_json) between 1 and 9
    ),
    created_at text not null
);

create trigger ai_overview_profile_runs_no_update
before update on ai_overview_profile_runs begin
    select raise(abort, 'AI overview profile contexts are immutable');
end;

create trigger ai_overview_profile_runs_no_delete
before delete on ai_overview_profile_runs begin
    select raise(abort, 'AI overview profile contexts are immutable');
end;
