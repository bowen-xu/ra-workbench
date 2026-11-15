create table agent_submission_images (
 conversation_id text not null,
 request_id text not null,
 source_id text not null,
 content_sha256 text not null,
 media_type text not null,
 byte_size integer not null check(byte_size > 0 and byte_size <= 20971520),
 ordinal integer not null check(ordinal >= 0),
 primary key(conversation_id, request_id, source_id),
 unique(conversation_id, request_id, ordinal),
 foreign key(conversation_id, request_id) references agent_submissions(conversation_id, request_id) on delete cascade
);
create index agent_submission_images_hash_idx on agent_submission_images(content_sha256);
