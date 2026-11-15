create table reading_profiles (
    id text not null,
    version integer not null check(version >= 1),
    profile_kind text not null check(profile_kind in (
        'generic', 'technical_empirical', 'philosophical_conceptual'
    )),
    label text not null check(length(label) between 1 and 120),
    description text not null check(length(description) between 1 and 1000),
    field_kinds_json text not null check(json_valid(field_kinds_json)),
    review_status text not null check(review_status = 'approved'),
    created_at text not null,
    primary key(id, version)
);

insert into reading_profiles values
    ('generic_overview', 1, 'generic', '通用阅读',
     '适合尚未确定论文类型时的基础阅读框架。',
     '["research_question","approach","key_result","conclusion","limitation","open_question"]',
     'approved', '2026-08-04T00:00:00Z'),
    ('technical_empirical', 1, 'technical_empirical', '技术／实证',
     '关注研究问题、方法或系统、关键结果、结论、局限和开放问题。',
     '["research_question","approach","key_result","conclusion","limitation","open_question"]',
     'approved', '2026-08-04T00:00:00Z'),
    ('philosophical_conceptual', 1, 'philosophical_conceptual', '哲学／概念',
     '关注核心问题、论证、异议、回应、结论和开放问题。',
     '["research_question","argument","objection","response","conclusion","open_question"]',
     'approved', '2026-08-04T00:00:00Z');

create table paper_reading_profile_selections (
    paper_id text primary key references bibliographic_items(id) on delete cascade,
    profile_refs_json text not null check(json_valid(profile_refs_json)),
    revision integer not null check(revision >= 1),
    created_at text not null,
    updated_at text not null
);

create table paper_reading_profile_selection_revisions (
    id text primary key,
    paper_id text not null,
    revision integer not null check(revision >= 1),
    profile_refs_json text not null check(json_valid(profile_refs_json)),
    change_reason text not null default '',
    created_at text not null,
    unique(paper_id, revision)
);

create trigger paper_reading_profile_selection_revisions_no_update
before update on paper_reading_profile_selection_revisions
begin
    select raise(abort, 'reading profile selection revisions are immutable');
end;

create trigger paper_reading_profile_selection_revisions_no_delete
before delete on paper_reading_profile_selection_revisions
begin
    select raise(abort, 'reading profile selection revisions are immutable');
end;
