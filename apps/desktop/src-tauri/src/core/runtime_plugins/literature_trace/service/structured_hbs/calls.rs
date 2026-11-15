//! Two finite pure calls share the existing host's request and input-file owner.
use super::*;
use research_workbench_native_sdk::editable_template::{
    EditableTemplateDecisionV1, EditableTemplateInputV1,
};
use research_workbench_native_sdk::review_selection::UnreviewedSelectionInputV1;

impl LiteratureTraceService {
    pub(super) async fn editable_template_decision(
        &self,
        slot: &mut Option<NativeHost>,
        key: &str,
        input: &EditableTemplateInputV1,
        before: &mut Option<(String, String)>,
    ) -> NativeResult<(Result<Value, String>, DecisionOrigin)> {
        let (output, origin) = self
            .structured_call(
                slot,
                key,
                CatalogInput::ValidateEditableTemplate {
                    input: Box::new(input.clone()),
                },
                before,
            )
            .await?;
        let CatalogOutput::EditableTemplate { outcome } = output else {
            return Err(output_error(slot, "可编辑模板校验结果种类不匹配。"));
        };
        if *outcome.input != *input {
            return Err(output_error(slot, "可编辑模板校验与原输入不匹配。"));
        }
        let result = match outcome.decision {
            EditableTemplateDecisionV1::Validated { canonical_json } => {
                let value: Value = serde_json::from_str(&canonical_json)
                    .map_err(|_| output_error(slot, "可编辑模板校验返回无效 JSON。"))?;
                let submitted: Value = serde_json::from_str(&input.candidate_json)
                    .map_err(|_| output_error(slot, "可编辑模板校验输入不是 JSON。"))?;
                // A transport binding, not a second domain validator. This pure
                // decision may accept or reject but must not rewrite the input.
                if value != submitted {
                    return Err(output_error(slot, "可编辑模板校验改变了原输入。"));
                }
                Ok(value)
            }
            EditableTemplateDecisionV1::Rejected { message } => Err(message),
        };
        Ok((result, origin))
    }
    async fn structured_call(
        &self,
        slot: &mut Option<NativeHost>,
        key: &str,
        input: CatalogInput,
        before: &mut Option<(String, String)>,
    ) -> NativeResult<(CatalogOutput, DecisionOrigin)> {
        self.available().map_err(|error| error.package(key))?;
        let stopped = self.stopped.notified();
        tokio::pin!(stopped);
        stopped.as_mut().enable();
        let host = self.ensure_host(slot, key).await?;
        *before = host.request_identity();
        self.available().map_err(|error| error.package(key))?;
        let output = tokio::select! { biased;
            _ = &mut stopped => Err(host.error("plugin_host_stopping", "应用正在关闭结构化校验。")),
            output = host.catalog_call(input, Duration::from_secs(60)) => output,
        }?;
        let (instance_id, native_request_id) = host.request_identity().ok_or_else(|| {
            host.error(
                "plugin_host_response_mismatch",
                "结构化校验缺少已完成请求身份。",
            )
        })?;
        Ok((
            output,
            DecisionOrigin {
                package_key: key.into(),
                instance_id,
                native_request_id,
            },
        ))
    }
    pub(super) async fn hbs_decision(
        &self,
        slot: &mut Option<NativeHost>,
        key: &str,
        input: &StructuredValidationInputV1,
        before: &mut Option<(String, String)>,
    ) -> NativeResult<(
        Result<Value, research_workbench_native_sdk::candidate_contracts::CandidateValidationError>,
        DecisionOrigin,
    )> {
        let (output, origin) = self
            .structured_call(
                slot,
                key,
                CatalogInput::ValidateStructured {
                    input: Box::new(input.clone()),
                },
                before,
            )
            .await?;
        let CatalogOutput::StructuredValidation { outcome } = output else {
            return Err(output_error(slot, "结构化校验结果种类不匹配。"));
        };
        let result = match *outcome {
            StructuredValidationOutcomeV1::Validated {
                input: actual,
                canonical_json,
            } if *actual == *input => Ok(serde_json::from_str(&canonical_json)
                .map_err(|_| output_error(slot, "结构化校验返回无效规范化 JSON。"))?),
            StructuredValidationOutcomeV1::Rejected {
                input: actual,
                error,
            } if *actual == *input => Err(error),
            _ => return Err(output_error(slot, "结构化校验与原输入不匹配。")),
        };
        Ok((result, origin))
    }
    pub(super) async fn unreviewed_selection(
        &self,
        slot: &mut Option<NativeHost>,
        key: &str,
        input: &UnreviewedSelectionInputV1,
        before: &mut Option<(String, String)>,
    ) -> NativeResult<Vec<String>> {
        let (output, _) = self
            .structured_call(
                slot,
                key,
                CatalogInput::SelectUnreviewedFields {
                    input: Box::new(input.clone()),
                },
                before,
            )
            .await?;
        let CatalogOutput::UnreviewedFields { selection } = output else {
            return Err(output_error(slot, "未校验字段选择结果种类不匹配。"));
        };
        if *selection.input != *input {
            return Err(output_error(slot, "未校验字段选择与原输入不匹配。"));
        }
        // The Core writer accepts only an ordered subset of its own pending
        // identities. Domain eligibility remains solely in the Native selector.
        let mut pending = input.pending_field_ids.iter();
        if selection
            .selected_field_ids
            .iter()
            .any(|field| !pending.by_ref().any(|id| id == field))
        {
            return Err(output_error(slot, "未校验字段选择超出本次字段范围。"));
        }
        Ok(selection.selected_field_ids)
    }
    pub(super) async fn finish_structured_failure(
        &self,
        slot: &mut Option<NativeHost>,
        key: &str,
        before: Option<(String, String)>,
        error: Box<crate::runtime_plugins::native::NativeHostError>,
        outcome: HbsOutcome,
    ) -> crate::runtime_plugins::literature_hbs::OperationError {
        match self.finish_domain::<()>(slot, Err(error)).await {
            Err(mut error) => {
                if let Some((instance, request)) = before.as_ref()
                    && error.instance_id.as_ref() == Some(instance)
                    && error.request_id.as_ref() == Some(request)
                {
                    error.request_id = None;
                }
                HbsOperationError::native(error.package(key), "validation", outcome).into()
            }
            Ok(()) => crate::runtime_plugins::literature_hbs::OperationError::unknown(),
        }
    }
}
fn output_error(
    slot: &Option<NativeHost>,
    message: &'static str,
) -> Box<crate::runtime_plugins::native::NativeHostError> {
    match slot {
        Some(host) => host.error("plugin_host_response_mismatch", message),
        None => LiteratureTraceService::error("plugin_host_response_mismatch", message),
    }
}
