-- Stable editable objects are distinct from immutable legacy AI candidates.
create table structured_resources (
    id text primary key,
    kind text not null check(kind in ('portrait','structuredReading','readingTemplate')),
    paper_id text references bibliographic_items(id) on delete cascade,
    title text not null,
    revision integer not null default 0 check(revision >= 0),
    content_json text check(content_json is null or json_valid(content_json)),
    baseline_json text check(baseline_json is null or json_valid(baseline_json)),
    reviewed_fields_json text not null default '[]' check(json_valid(reviewed_fields_json)),
    protected_fields_json text not null default '[]' check(json_valid(protected_fields_json)),
    template_id text references structured_resources(id),
    template_revision integer,
    created_at text not null default(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    updated_at text not null default(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    unique(paper_id,kind),
    check((kind = 'readingTemplate' and paper_id is null) or (kind != 'readingTemplate' and paper_id is not null))
);
create table structured_resource_revisions (
    resource_id text not null references structured_resources(id) on delete cascade,
    revision integer not null check(revision >= 1),
    content_json text check(content_json is null or json_valid(content_json)),
    baseline_json text check(baseline_json is null or json_valid(baseline_json)),
    reviewed_fields_json text not null check(json_valid(reviewed_fields_json)),
    protected_fields_json text not null check(json_valid(protected_fields_json)),
    editor_kind text not null check(editor_kind in ('human','agent','migration','review')),
    source_snapshot_id text,
    template_id text,
    template_revision integer,
    created_at text not null default(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    primary key(resource_id,revision)
);
create table structured_resource_drafts (
    id text primary key,
    resource_id text not null references structured_resources(id) on delete cascade,
    creator_scope text not null,
    creator_conversation_id text,
    base_revision integer not null check(base_revision >= 0),
    revision integer not null check(revision >= 1),
    content text not null,
    content_hash text not null check(length(content_hash) = 64),
    source_snapshot_id text,
    errors_json text not null default '[]' check(json_valid(errors_json)),
    updated_at text not null default(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    unique(resource_id,creator_scope)
);
create table structured_resource_receipts (
    request_id text primary key,
    resource_id text not null references structured_resources(id) on delete cascade,
    revision integer not null check(revision >= 0),
    content_hash text not null,
    operation text not null,
    request_hash text not null,
    conversation_id text,
    submission_id text,
    created_at text not null default(strftime('%Y-%m-%dT%H:%M:%fZ','now')),
    foreign key(resource_id,revision) references structured_resource_revisions(resource_id,revision)
);
create index structured_resource_receipts_submission on structured_resource_receipts(conversation_id,submission_id,resource_id);
create table structured_resource_operations (
    request_id text primary key,
    resource_id text not null references structured_resources(id) on delete cascade,
    request_hash text not null,
    result_json text not null check(json_valid(result_json)),
    conversation_id text,
    submission_id text,
    created_at text not null default(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
create table structured_resource_creations (
    request_id text primary key,
    resource_id text not null references structured_resources(id) on delete cascade,
    creator_conversation_id text
);
create table structured_resource_legacy_imports (
    import_key text primary key,
    imported_at text not null default(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
create table structured_resource_legacy_versions (
    legacy_id text not null,
    legacy_revision integer not null,
    resource_id text not null references structured_resources(id) on delete cascade,
    payload_json text not null check(json_valid(payload_json)),
    primary key(legacy_id,legacy_revision)
);
create table agent_context_configurations (
    conversation_id text primary key references agent_conversations(id) on delete cascade,
    revision integer not null check(revision >= 1),
    state_json text not null check(json_valid(state_json)),
    updated_at text not null default(strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
insert into structured_resources(id,kind,paper_id,title)
select lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(6))),'portrait',id,'文献画像' from bibliographic_items;
insert into structured_resources(id,kind,paper_id,title)
select lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(6))),'structuredReading',id,'结构化阅读' from bibliographic_items;
create trigger bibliographic_structured_resources_created after insert on bibliographic_items begin
    insert into structured_resources(id,kind,paper_id,title) values(lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(6))),'portrait',new.id,'文献画像');
    insert into structured_resources(id,kind,paper_id,title) values(lower(hex(randomblob(4)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(2)))||'-'||lower(hex(randomblob(6))),'structuredReading',new.id,'结构化阅读');
end;
