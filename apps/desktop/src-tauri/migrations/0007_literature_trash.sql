alter table bibliographic_items add column trashed_at text;

create index bibliographic_items_trashed_at_idx
    on bibliographic_items(trashed_at);
