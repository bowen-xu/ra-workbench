-- Older applications cannot decode the interleaved-output event. The numbered
-- migration makes their startup reject this schema before replay or mutation.
-- Startup materializes the pending v217 log before recording this fence.
select 1;
