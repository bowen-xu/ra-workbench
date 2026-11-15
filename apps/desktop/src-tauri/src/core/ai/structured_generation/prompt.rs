//! Shared composer-compatible skill identity binding. Task interpretation is domain-owned.
use research_agent_core::user_input::{TextElement, UserInput};

/// Preserve composer bindings while explicitly selecting the versioned skill.
pub(in crate::ai) fn bind_skill(
    input: &mut research_agent_core::submission::SubmissionInput,
    skill: &research_agent_core::skills::SkillDescriptor,
) {
    let prefix = format!("按照 @{} 执行本次任务。\n\n", skill.name);
    let offset = prefix.encode_utf16().count();
    let item_index = input.user_input.len().max(1);
    let mut elements = vec![TextElement {
        start_utf16: "按照 ".encode_utf16().count(),
        end_utf16: "按照 ".encode_utf16().count() + 1 + skill.name.encode_utf16().count(),
        item_index,
    }];
    if let Some(UserInput::Text { text_elements, .. }) = input.user_input.first() {
        elements.extend(text_elements.iter().cloned().map(|mut element| {
            element.start_utf16 += offset;
            element.end_utf16 += offset;
            element
        }));
    }
    input.text.insert_str(0, &prefix);
    let first = UserInput::Text {
        text: input.text.clone(),
        text_elements: elements,
    };
    if input.user_input.is_empty() {
        input.user_input.push(first);
    } else {
        input.user_input[0] = first;
    }
    input.user_input.push(UserInput::Skill {
        name: skill.name.clone(),
        identity: skill.identity.clone(),
    });
}

#[cfg(test)]
#[path = "prompt/tests.rs"]
mod tests;
