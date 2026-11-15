-- New runs reference a global template version; legacy portrait snapshots stay intact.
create table ai_resource_template_runs (
    run_id text primary key references ai_runs(id) on delete cascade,
    paper_id text not null references bibliographic_items(id) on delete cascade,
    template_id text not null references structured_resources(id),
    template_revision integer not null,
    selection_revision integer not null,
    template_json text not null check(json_valid(template_json)),
    foreign key(template_id,template_revision)
        references structured_resource_revisions(resource_id,revision)
);
