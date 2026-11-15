-- A reviewed manual compact keeps its original history fence across reopen.
-- The submission remains the authority for frozen model, source and request IDs.
create table agent_compaction_preparations (
    conversation_id text not null,
    request_id text not null,
    expected_revision integer not null check(expected_revision >= 0 and expected_revision <= 9007199254740991),
    primary key(conversation_id, request_id),
    foreign key(conversation_id, request_id)
        references agent_submissions(conversation_id, request_id) on delete cascade
);
