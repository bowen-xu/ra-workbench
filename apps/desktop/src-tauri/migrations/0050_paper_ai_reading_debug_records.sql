alter table paper_ai_reading_turns
    add column raw_request_json text check(raw_request_json is null or json_valid(raw_request_json));

alter table paper_ai_reading_turns
    add column raw_response_json text check(raw_response_json is null or json_valid(raw_response_json));
