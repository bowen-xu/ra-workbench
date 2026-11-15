-- Snapshot v3 retains structured user input; v4 also retains individually held
-- inputs. Keep the deployed terminal-pause behavior and update only its version
-- allowlist. json_set preserves payloads, origins, held inputs and active claims.
drop trigger agent_terminal_queue_pause;
create trigger agent_terminal_queue_pause
 after insert on agent_journal_records
 when json_extract(new.payload_json, '$.payload.type') = 'turnFinished'
  and json_extract(new.payload_json, '$.payload.outcome') in ('failed', 'canceled', 'interrupted')
 begin
    select case when exists (
        select 1 from agent_input_queues
        where conversation_id = new.conversation_id
          and (coalesce(json_extract(snapshot_json, '$.version'), -1) not in (1, 2, 3, 4)
            or json_extract(snapshot_json, '$.conversation') is not new.conversation_id
            or json_extract(snapshot_json, '$.revision') is not revision
            or coalesce(json_type(snapshot_json, '$.paused'), 'missing') not in ('true', 'false')
            or (json_extract(snapshot_json, '$.paused') = 0 and revision >= 9223372036854775807))
    ) then raise(abort, 'invalid queue state during terminal commit') end;
    insert into agent_input_queues(conversation_id, revision, snapshot_json)
    values(new.conversation_id, 1, json_object(
        'version', 1, 'conversation', new.conversation_id, 'revision', 1,
        'paused', json('true'), 'inputs', json_array(), 'preparing', null))
    on conflict(conversation_id) do update set
        revision = agent_input_queues.revision + 1,
        snapshot_json = json_set(agent_input_queues.snapshot_json,
            '$.revision', agent_input_queues.revision + 1, '$.paused', json('true'))
    where json_extract(agent_input_queues.snapshot_json, '$.paused') = 0;
 end;
