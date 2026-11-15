create table agent_queue_submission_origins (
    conversation_id text not null,
    request_id text not null,
    input_id text not null references agent_input_queue_identities(input_id) on delete cascade,
    primary key (conversation_id, request_id),
    foreign key (conversation_id, request_id)
        references agent_submissions(conversation_id, request_id) on delete cascade
);
insert into agent_queue_submission_origins(conversation_id,request_id,input_id)
select conversation_id,request_id,input_id from agent_input_queue_previews
union
select conversation_id,request_id,input_id from agent_input_queue_admissions;
-- Cancelled previews retain submissions after their active preview link is removed.
insert or ignore into agent_queue_submission_origins(conversation_id,request_id,input_id)
select r.conversation_id,json_extract(r.command_json,'$[2][2]'),json_extract(r.command_json,'$[2][1]')
from agent_input_queue_receipts r
join agent_submissions s on s.conversation_id=r.conversation_id
    and s.request_id=json_extract(r.command_json,'$[2][2]')
where json_extract(r.command_json,'$[2][0]')='cancelPreview';
