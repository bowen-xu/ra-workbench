alter table paper_ai_reading_turns
    add column messages_json text not null default '[]'
        check(json_valid(messages_json) and json_type(messages_json) = 'array');

update paper_ai_reading_turns
set messages_json = case
    when length(trim(assistant_message)) = 0 then json_array(
        json_object(
            'id', id || '-user-0',
            'role', 'user',
            'content', user_message
        )
    )
    else json_array(
        json_object(
            'id', id || '-user-0',
            'role', 'user',
            'content', user_message
        ),
        json_object(
            'id', id || '-assistant-0',
            'role', 'assistant',
            'content', assistant_message
        )
    )
end;
