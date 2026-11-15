-- Search visibility follows the same surviving turns as the core projection.
-- These derived tombstones never remove immutable journal or message evidence.
-- Rust backfills deployed rollback ledgers inside this migration transaction.
create table agent_rolled_back_turns (
    conversation_id text not null references agent_conversations(id) on delete cascade,
    turn_id text not null,
    rollback_ordinal integer not null check(rollback_ordinal >= 0),
    primary key(conversation_id, turn_id)
);
