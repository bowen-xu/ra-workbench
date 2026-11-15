-- Keep the original queue and steering identities across process termination.
create table agent_steering_handoffs (
    steering_input_id text primary key not null,
    conversation_id text not null references agent_conversations(id) on delete cascade,
    queue_input_id text not null,
    operation_id text not null unique,
    epoch text not null,
    intent_json text not null check(json_valid(intent_json)),
    unique(conversation_id, queue_input_id)
);
