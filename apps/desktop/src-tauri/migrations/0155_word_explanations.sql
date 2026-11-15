create table word_explanations (
    id text primary key,
    attachment_id text not null references attachments(id) on delete cascade,
    content_sha256 text not null check(length(content_sha256) = 64),
    payload_json text not null check(json_valid(payload_json)),
    updated_at text not null
);
create index word_explanations_attachment_idx on word_explanations(attachment_id, content_sha256);
