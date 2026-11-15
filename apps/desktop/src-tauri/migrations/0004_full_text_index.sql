create table attachment_page_texts (
    attachment_id text not null references attachments(id) on delete cascade,
    page_index integer not null check(page_index >= 0),
    page_text text not null,
    indexed_at text not null,
    primary key (attachment_id, page_index)
);

create table full_text_index_jobs (
    attachment_id text primary key references attachments(id) on delete cascade,
    state text not null check(state in ('running', 'completed', 'failed', 'interrupted')),
    page_count integer not null default 0,
    error_message text,
    updated_at text not null
);
