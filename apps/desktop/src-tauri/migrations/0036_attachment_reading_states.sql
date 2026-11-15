create table attachment_reading_states (
    attachment_id text primary key references attachments(id) on delete cascade,
    item_id text not null references bibliographic_items(id) on delete cascade,
    page_index integer not null default 0 check(page_index >= 0),
    zoom real not null default 1 check(zoom > 0),
    updated_at text not null
);

insert into attachment_reading_states (attachment_id, item_id, page_index, zoom, updated_at)
select state.attachment_id, attachment.item_id, state.page_index, state.zoom, state.updated_at
from reading_states state
join attachments attachment on attachment.id = state.attachment_id
where state.rowid = coalesce(
    (
        select candidate.rowid
        from reading_states candidate
        where candidate.attachment_id = state.attachment_id
          and candidate.item_id = attachment.item_id
        order by candidate.updated_at desc
        limit 1
    ),
    (
        select candidate.rowid
        from reading_states candidate
        where candidate.attachment_id = state.attachment_id
        order by candidate.updated_at desc
        limit 1
    )
);

drop table reading_states;
alter table attachment_reading_states rename to reading_states;
create index reading_states_item_id_idx on reading_states(item_id);
