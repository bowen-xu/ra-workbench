-- Initial windows have no checkpoint. Keep their identity independently so
-- reopens and rollback before the first checkpoint restore the same window.
create table agent_initial_context_windows (
    conversation_id text primary key not null references agent_conversations(id) on delete cascade,
    window_id text not null unique,
    created_at text not null default (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
create trigger agent_initial_context_windows_immutable
before update on agent_initial_context_windows
begin
    select raise(abort, 'initial context windows are immutable');
end;
