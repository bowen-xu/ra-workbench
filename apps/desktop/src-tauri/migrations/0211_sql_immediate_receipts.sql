-- SQL-only operations share the global operation namespace, but not log positions.
create table data_sql_operation_receipts (
    operation_id text primary key not null,
    epoch text not null,
    kind text not null check(length(kind) > 0),
    version integer not null check(version > 0),
    fingerprint blob not null check(typeof(fingerprint) = 'blob' and length(fingerprint) = 32),
    sql_fence_position integer not null check(sql_fence_position >= 0),
    result_json text not null check(json_valid(result_json))
);

-- No FK: deleting a conversation must not release a known operation UUID for
-- reuse by a different command family. Retain original batch identity evidence.
create table data_journal_operation_ids (
    conversation_id text not null,
    batch_id text not null,
    operation_id text not null unique,
    primary key(conversation_id, batch_id),
    check(operation_id = data_journal_operation_id(conversation_id, batch_id))
);
insert into data_journal_operation_ids(conversation_id, batch_id, operation_id)
select conversation_id, batch_id, data_journal_operation_id(conversation_id, batch_id)
from agent_journal_batches;

create trigger data_sql_receipt_identity_guard before insert on data_sql_operation_receipts
when exists(select 1 from data_journal_operation_ids where operation_id = new.operation_id)
  or exists(select 1 from data_operation_receipts where operation_id = new.operation_id)
begin
    select raise(abort, 'DATA_OPERATION_KIND_CONFLICT');
end;

create trigger data_journal_batch_identity_guard before insert on agent_journal_batches
when exists(select 1 from data_sql_operation_receipts
    where operation_id = data_journal_operation_id(new.conversation_id, new.batch_id))
begin
    select raise(abort, 'DATA_OPERATION_KIND_CONFLICT');
end;

create trigger data_journal_operation_identity_insert after insert on agent_journal_batches
begin
    insert into data_journal_operation_ids(conversation_id, batch_id, operation_id)
    values(new.conversation_id, new.batch_id,
        data_journal_operation_id(new.conversation_id, new.batch_id))
    on conflict(conversation_id, batch_id) do nothing;
end;

create trigger data_log_receipt_identity_guard before insert on data_operation_receipts
when exists(select 1 from data_sql_operation_receipts where operation_id = new.operation_id)
begin
    select raise(abort, 'DATA_OPERATION_KIND_CONFLICT');
end;
