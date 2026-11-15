-- Field-level provenance retains mixed legacy selections and precise Undo sources.
create table structured_resource_field_sources (
    resource_id text not null,
    revision integer not null,
    field_id text not null,
    source_snapshot_id text not null,
    primary key(resource_id,revision,field_id,source_snapshot_id),
    foreign key(resource_id,revision) references structured_resource_revisions(resource_id,revision) on delete cascade
);
