-- Durable compaction checkpoints. The journal remains authoritative; this table
-- records request projections and summaries for recovery and stale-result checks.
create table agent_compactions (
    id text primary key,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    turn_id text not null,
    history_sha256 text not null check(length(history_sha256)=64),
    original_message_count integer not null check(original_message_count >= 0),
    retained_message_count integer not null check(retained_message_count >= 1),
    omitted_message_count integer not null check(omitted_message_count >= 0),
    original_bytes integer not null check(original_bytes >= 0),
    retained_bytes integer not null check(retained_bytes >= 0),
    summary_input text not null,
    summary_text text,
    state text not null default 'projected' check(state in ('projected','summarized','failed')),
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
create index agent_compactions_conversation_idx
    on agent_compactions(conversation_id, created_at desc, id desc);
