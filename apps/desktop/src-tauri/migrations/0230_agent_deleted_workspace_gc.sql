-- A committed conversation deletion may leave execution copies to discard.
-- Keep the frozen identity independently of the cascading conversation rows.
create table agent_deleted_workspace_gc (
    operation_id text not null,
    conversation_id text not null,
    turn_id text not null,
    manuscript_id text not null,
    lease_id text not null,
    primary key (operation_id, lease_id)
);
