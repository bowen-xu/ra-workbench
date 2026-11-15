use super::*;
use research_agent_core::{
    model::ReasoningSummary,
    protocol::{ReasoningSummaryIndex, ReasoningSummaryPart},
};

pub(super) fn capture(
    template: &mut Template,
    item_id: String,
) -> Result<ReasoningSummary, ModelError> {
    let mut result = ReasoningSummary {
        item_id,
        text: String::new(),
        parts: vec![],
    };
    // Preserve unknown arrays in full. A readable subset cannot stand in for
    // the complete provider fact or manufacture canonical array positions.
    let recognized = template.item["summary"].as_array().is_some_and(|parts| {
        parts
            .iter()
            .all(|part| part["type"] == "summary_text" && part["text"].is_string())
    });
    if !recognized {
        return Ok(result);
    }
    template.summary_from_common = true;
    let parts = template.item["summary"]
        .as_array_mut()
        .ok_or(ModelError::InvalidStream)?;
    for (index, part) in parts.iter_mut().enumerate() {
        let text = part
            .as_object_mut()
            .ok_or(ModelError::InvalidStream)?
            .remove("text")
            .and_then(|value| value.as_str().map(str::to_owned))
            .ok_or(ModelError::InvalidStream)?;
        result.parts.push(ReasoningSummaryPart {
            index: ReasoningSummaryIndex::from(
                i64::try_from(index).map_err(|_| ModelError::BudgetExceeded)?,
            ),
            start_byte: result.text.len(),
            len_byte: text.len(),
        });
        result.text.push_str(&text);
    }
    Ok(result)
}

pub(super) fn render(
    template: &mut Template,
    summary: &ReasoningSummary,
) -> Result<(), ModelError> {
    if template.namespace_from_common || !template.text_bindings.is_empty() {
        return Err(ModelError::InvalidStream);
    }
    if !template.summary_from_common {
        return if summary.text.is_empty() && summary.parts.is_empty() {
            Ok(())
        } else {
            Err(ModelError::InvalidStream)
        };
    }
    let parts = template.item["summary"]
        .as_array_mut()
        .ok_or(ModelError::InvalidStream)?;
    if parts.len() != summary.parts.len() {
        return Err(ModelError::InvalidStream);
    }
    for (index, (part, binding)) in parts.iter_mut().zip(&summary.parts).enumerate() {
        if part["type"] != "summary_text"
            || part.get("text").is_some()
            || binding.index.as_str() != index.to_string()
        {
            return Err(ModelError::InvalidStream);
        }
        let end = binding
            .start_byte
            .checked_add(binding.len_byte)
            .ok_or(ModelError::InvalidStream)?;
        part["text"] = Value::String(
            summary
                .text
                .get(binding.start_byte..end)
                .ok_or(ModelError::InvalidStream)?
                .into(),
        );
    }
    Ok(())
}
