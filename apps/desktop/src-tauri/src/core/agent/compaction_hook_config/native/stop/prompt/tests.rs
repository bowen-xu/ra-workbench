use super::*;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(rename = "hook_prompt")]
struct HookPrompt {
    #[serde(rename = "@hook_run_id")]
    run_id: String,
    #[serde(rename = "$text")]
    text: String,
}
fn blocked(run: Uuid, text: &str) -> (Uuid, Uuid, Outcome) {
    (
        Uuid::new_v4(),
        run,
        Outcome::Completed {
            status: Status::Blocked {
                feedback: text.into(),
            },
            system_message: None,
        },
    )
}
#[test]
fn preserves_each_run_identity_and_escapes_xml_in_declaration_order() {
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let StopResolution::FollowUp { parts } = resolution(
        false,
        &[
            blocked(first, "retry <one> & \"two\""),
            blocked(second, "第二项"),
        ],
    ) else {
        panic!("follow-up expected");
    };
    assert_eq!(
        parts.iter().map(|part| part.run_id).collect::<Vec<_>>(),
        vec![first, second]
    );
    let parsed: Vec<HookPrompt> = parts
        .iter()
        .map(|part| quick_xml::de::from_str(&part.input_text).unwrap())
        .collect();
    assert_eq!(parsed[0].run_id, first.to_string());
    assert_eq!(parsed[0].text, "retry <one> & \"two\"");
    assert_eq!(parsed[1].run_id, second.to_string());
    assert_eq!(parsed[1].text, "第二项");
}
#[test]
fn stop_and_cancellation_never_create_continuation() {
    let stop = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Outcome::Completed {
            status: Status::Stopped {
                reason: Some("done".into()),
            },
            system_message: None,
        },
    );
    let outcomes = vec![blocked(Uuid::new_v4(), "retry"), stop];
    assert_eq!(
        resolution(false, &outcomes),
        StopResolution::Stop {
            reason: Some("done".into())
        }
    );
    assert_eq!(resolution(true, &outcomes), StopResolution::Canceled);
    assert_eq!(resolution(false, &[]), StopResolution::Continue);
}
