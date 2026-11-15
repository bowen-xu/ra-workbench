use super::*;
use crate::runtime_plugins::literature_nal_bootstrap::{
    BootstrapCause, BootstrapProblem, FailureContext,
};
use research_workbench_native_sdk::nal_bootstrap::{
    BootstrapIdentitiesV1, BootstrapPhaseV1, BootstrapProbePreparationV1, BootstrapSourceV1,
    ConceptIdentitySlotsV1, SourceProbe,
};
use uuid::Uuid;

/// These are physical bounds of this closed input shape, not relation filtering
/// or direction policy. No legal source gains a new result or business limit.
pub(super) fn allocate(
    source: &BootstrapSourceV1,
    probes: &BootstrapProbePreparationV1,
) -> NativeResult<BootstrapIdentitiesV1> {
    let invalid = || {
        LiteratureTraceService::error(
            "plugin_host_response_mismatch",
            "关系网络准备的身份数量不符合本次输入。",
        )
    };
    let statements = len(&source.relations)
        .checked_mul(2)
        .and_then(|n| n.checked_add(len(&source.collections)))
        .and_then(|n| n.checked_add(len(&source.memberships)))
        .and_then(|n| n.checked_add(len(&source.assignments)))
        .ok_or_else(invalid)?;
    // Five singleton ConceptCallKey variants plus the two source-keyed variants.
    let concepts = 5_usize
        .checked_add(len(&source.collections))
        .and_then(|n| n.checked_add(len(&source.relation_types)))
        .ok_or_else(invalid)?;
    if probes.statement_count > statements || probes.concepts.len() > concepts {
        return Err(invalid());
    }
    let mut slots = Vec::new();
    slots
        .try_reserve_exact(probes.concepts.len())
        .map_err(allocation)?;
    for concept in &probes.concepts {
        slots.push(ConceptIdentitySlotsV1 {
            key: concept.key.clone(),
            term_id: id(),
            term_revision_id: id(),
            concept_id: id(),
            concept_revision_id: id(),
            representation_id: id(),
            record_id: id(),
            job_id: id(),
        });
    }
    let mut statement_ids = Vec::new();
    statement_ids
        .try_reserve_exact(probes.statement_count)
        .map_err(allocation)?;
    statement_ids.extend((0..probes.statement_count).map(|_| id()));
    Ok(BootstrapIdentitiesV1 {
        concepts: slots,
        generation_id: Some(id()),
        statement_ids,
    })
}

fn len<T>(value: &SourceProbe<Vec<T>>) -> usize {
    match value {
        SourceProbe::Available { value } => value.len(),
        _ => 0,
    }
}

fn allocation(cause: std::collections::TryReserveError) -> Box<NativeHostError> {
    let mut error = LiteratureTraceService::error(
        "storage_error",
        format!("关系网络身份内存分配失败：{cause}"),
    );
    error.bootstrap_problem = Some(Box::new(BootstrapProblem {
        phase: BootstrapPhaseV1::LegacyHeader,
        context: FailureContext::Network,
        cause: BootstrapCause::Owner("identity_slot_allocation_failed"),
    }));
    error
}

fn id() -> String {
    Uuid::new_v4().to_string()
}
