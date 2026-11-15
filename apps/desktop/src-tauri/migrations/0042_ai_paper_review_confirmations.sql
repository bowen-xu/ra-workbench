create table ai_paper_review_confirmation_batches (
    id text primary key,
    paper_id text not null references bibliographic_items(id) on delete restrict,
    project_id text references research_projects(id) on delete restrict,
    request_json text not null check(json_valid(request_json)),
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    created_at text not null
);

create table ai_paper_review_metadata_confirmations (
    id text primary key,
    batch_id text not null references ai_paper_review_confirmation_batches(id) on delete restrict,
    field_name text not null,
    previous_confirmed_by_user integer not null check(previous_confirmed_by_user in (0, 1)),
    previous_updated_at text not null,
    applied_updated_at text not null,
    created_at text not null,
    unique(batch_id, field_name)
);

create table ai_paper_review_overview_batch_links (
    batch_id text not null references ai_paper_review_confirmation_batches(id) on delete restrict,
    overview_batch_id text not null unique references ai_overview_confirmation_batches(id) on delete restrict,
    created_at text not null,
    primary key(batch_id, overview_batch_id)
);

create table ai_paper_review_metadata_confirmation_undos (
    id text primary key,
    confirmation_id text not null unique references ai_paper_review_metadata_confirmations(id) on delete restrict,
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    created_at text not null
);

create table ai_paper_review_confirmation_undos (
    id text primary key,
    batch_id text not null unique references ai_paper_review_confirmation_batches(id) on delete restrict,
    idempotency_key text not null unique check(length(idempotency_key) between 1 and 200),
    created_at text not null
);

create trigger ai_paper_review_confirmation_batches_no_update
before update on ai_paper_review_confirmation_batches
begin
    select raise(abort, 'AI paper review confirmation batches are immutable');
end;

create trigger ai_paper_review_confirmation_batches_no_delete
before delete on ai_paper_review_confirmation_batches
begin
    select raise(abort, 'AI paper review confirmation batches are immutable');
end;

create trigger ai_paper_review_metadata_confirmations_no_update
before update on ai_paper_review_metadata_confirmations
begin
    select raise(abort, 'AI paper review metadata confirmations are immutable');
end;

create trigger ai_paper_review_metadata_confirmations_no_delete
before delete on ai_paper_review_metadata_confirmations
begin
    select raise(abort, 'AI paper review metadata confirmations are immutable');
end;

create trigger ai_paper_review_overview_batch_links_no_update
before update on ai_paper_review_overview_batch_links
begin
    select raise(abort, 'AI paper review overview links are immutable');
end;

create trigger ai_paper_review_overview_batch_links_no_delete
before delete on ai_paper_review_overview_batch_links
begin
    select raise(abort, 'AI paper review overview links are immutable');
end;

create trigger ai_paper_review_metadata_confirmation_undos_no_update
before update on ai_paper_review_metadata_confirmation_undos
begin
    select raise(abort, 'AI paper review metadata undos are immutable');
end;

create trigger ai_paper_review_metadata_confirmation_undos_no_delete
before delete on ai_paper_review_metadata_confirmation_undos
begin
    select raise(abort, 'AI paper review metadata undos are immutable');
end;

create trigger ai_paper_review_confirmation_undos_no_update
before update on ai_paper_review_confirmation_undos
begin
    select raise(abort, 'AI paper review undos are immutable');
end;

create trigger ai_paper_review_confirmation_undos_no_delete
before delete on ai_paper_review_confirmation_undos
begin
    select raise(abort, 'AI paper review undos are immutable');
end;

create index ai_paper_review_confirmation_batches_paper_idx
    on ai_paper_review_confirmation_batches(paper_id, created_at desc);
