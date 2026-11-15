-- Connector receipts outlive deleted library entities, so a transport retry can
-- never recreate a removed item or overwrite subsequent manual edits.
create table browser_capture_sessions (
    scope text not null,
    session_id text not null,
    operation text not null check(operation in ('items', 'webpage', 'standalone')),
    request_sha256 text not null check(length(request_sha256) = 64),
    payload_json text not null check(json_valid(payload_json)),
    receipt_json text not null check(json_valid(receipt_json)),
    created_at text not null,
    primary key (scope, session_id)
);

create table browser_capture_items (
    scope text not null,
    session_id text not null,
    external_id text not null,
    item_id text references bibliographic_items(id) on delete set null,
    original_item_type text not null,
    raw_json text not null check(json_valid(raw_json)),
    primary key (scope, session_id, external_id),
    foreign key (scope, session_id)
        references browser_capture_sessions(scope, session_id) on delete cascade
);

create table browser_capture_attachments (
    scope text not null,
    session_id text not null,
    external_id text not null,
    item_id text references bibliographic_items(id) on delete set null,
    attachment_id text references attachments(id) on delete set null,
    request_sha256 text not null check(length(request_sha256) = 64),
    metadata_json text not null check(json_valid(metadata_json)),
    content_sha256 text not null check(length(content_sha256) = 64),
    receipt_json text not null check(json_valid(receipt_json)),
    created_at text not null,
    primary key (scope, session_id, external_id),
    foreign key (scope, session_id)
        references browser_capture_sessions(scope, session_id) on delete cascade
);

create index browser_capture_items_item_idx on browser_capture_items(item_id);
create index browser_capture_attachments_item_idx on browser_capture_attachments(item_id);
create index browser_capture_attachments_attachment_idx
    on browser_capture_attachments(attachment_id);
