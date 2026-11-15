-- Existing proposals keep deferred-save recovery; new deliveries checkpoint
-- current D and review B together while advancing their separate saved CAS S.
alter table agent_note_patch_candidates add column persistence_version integer
 not null default 1 check(persistence_version in (1,2));
