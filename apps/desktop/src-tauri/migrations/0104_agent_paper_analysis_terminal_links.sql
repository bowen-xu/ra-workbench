alter table agent_paper_analysis_submissions
    add column ai_run_id text references ai_runs(id) on delete set null;

alter table agent_paper_analysis_submissions
    add column candidate_id text references ai_candidates(id) on delete set null;

alter table agent_paper_analysis_submissions
    add column terminal_status text;

alter table agent_paper_analysis_submissions
    add column updated_at text;

create index agent_paper_analysis_submissions_terminal_idx
    on agent_paper_analysis_submissions(terminal_status, updated_at);
