use std::{collections::HashMap, env, path::PathBuf};

use research_assistant_lib::{CalibrationPricing, CalibrationRunOptions, run_calibration};

fn main() {
    let result = match parse_options() {
        Ok(options) => tauri::async_runtime::block_on(run_calibration(options)),
        Err(message) => Err(message),
    };
    match result {
        Ok(case_count) => {
            println!("Wrote calibration run artifact for {case_count} synthetic cases")
        }
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}

fn parse_options() -> Result<CalibrationRunOptions, String> {
    let values = parse_pairs(env::args().skip(1).collect())?;
    let pricing = parse_pricing(&values)?;
    Ok(CalibrationRunOptions {
        scenarios_path: required_path(&values, "--scenarios")?,
        output_path: required_path(&values, "--output")?,
        credentials_path: required_path(&values, "--credentials")?,
        case_id: values.get("--case").cloned(),
        base_url: values
            .get("--base-url")
            .cloned()
            .unwrap_or_else(|| "https://api.deepseek.com".to_owned()),
        model_id: required(&values, "--model")?.to_owned(),
        timeout_ms: optional_number(&values, "--timeout-ms", 60_000)?,
        max_output_tokens: optional_number(&values, "--max-output-tokens", 2_000)?,
        pricing,
    })
}

fn parse_pairs(arguments: Vec<String>) -> Result<HashMap<String, String>, String> {
    const ALLOWED: [&str; 14] = [
        "--scenarios",
        "--output",
        "--credentials",
        "--case",
        "--model",
        "--base-url",
        "--timeout-ms",
        "--max-output-tokens",
        "--input-cost-microusd-per-million",
        "--cache-hit-cost-microusd-per-million",
        "--cache-miss-cost-microusd-per-million",
        "--output-cost-microusd-per-million",
        "--cost-basis",
        "--confirmation",
    ];
    if !arguments.len().is_multiple_of(2) {
        return Err(usage());
    }
    let mut values = HashMap::new();
    for pair in arguments.chunks_exact(2) {
        if !ALLOWED.contains(&pair[0].as_str())
            || values.insert(pair[0].clone(), pair[1].clone()).is_some()
        {
            return Err(usage());
        }
    }
    if values.get("--confirmation").map(String::as_str) != Some("synthetic-data-may-incur-cost") {
        return Err("必须显式提供 --confirmation synthetic-data-may-incur-cost".to_owned());
    }
    Ok(values)
}

fn parse_pricing(values: &HashMap<String, String>) -> Result<Option<CalibrationPricing>, String> {
    const KEYS: [&str; 5] = [
        "--input-cost-microusd-per-million",
        "--cache-hit-cost-microusd-per-million",
        "--cache-miss-cost-microusd-per-million",
        "--output-cost-microusd-per-million",
        "--cost-basis",
    ];
    let present = KEYS.iter().filter(|key| values.contains_key(**key)).count();
    if present == 0 {
        return Ok(None);
    }
    if present != KEYS.len() {
        return Err("价格参数必须五项同时提供；未知成本请全部省略".to_owned());
    }
    Ok(Some(CalibrationPricing {
        input_microusd_per_million: required_number(values, "--input-cost-microusd-per-million")?,
        cache_hit_microusd_per_million: required_number(
            values,
            "--cache-hit-cost-microusd-per-million",
        )?,
        cache_miss_microusd_per_million: required_number(
            values,
            "--cache-miss-cost-microusd-per-million",
        )?,
        output_microusd_per_million: required_number(values, "--output-cost-microusd-per-million")?,
        cost_basis: required(values, "--cost-basis")?.to_owned(),
    }))
}

fn required_path(values: &HashMap<String, String>, key: &str) -> Result<PathBuf, String> {
    Ok(PathBuf::from(required(values, key)?))
}

fn required<'a>(values: &'a HashMap<String, String>, key: &str) -> Result<&'a str, String> {
    values
        .get(key)
        .filter(|value| !value.trim().is_empty())
        .map(String::as_str)
        .ok_or_else(usage)
}

fn required_number<T>(values: &HashMap<String, String>, key: &str) -> Result<T, String>
where
    T: std::str::FromStr,
{
    required(values, key)?
        .parse()
        .map_err(|_| format!("{key} 必须是非负整数"))
}

fn optional_number<T>(values: &HashMap<String, String>, key: &str, fallback: T) -> Result<T, String>
where
    T: std::str::FromStr,
{
    values.get(key).map_or(Ok(fallback), |value| {
        value.parse().map_err(|_| format!("{key} 必须是非负整数"))
    })
}

fn usage() -> String {
    "用法：pnpm ai:calibration:run -- --scenarios <json> --output <new-json> --credentials <exact-json-path> --model <model-id> --confirmation synthetic-data-may-incur-cost [--case <exact-case-id>] [--base-url <official-url>] [--timeout-ms <ms>] [--max-output-tokens <n>]；价格五项参数必须同时提供".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_is_all_or_nothing() {
        let partial = HashMap::from([(
            "--input-cost-microusd-per-million".to_owned(),
            "100".to_owned(),
        )]);
        assert!(parse_pricing(&partial).is_err());
        assert!(
            parse_pricing(&HashMap::new())
                .expect("unknown pricing")
                .is_none()
        );
    }

    #[test]
    fn case_selector_is_an_explicit_single_value() {
        let values = parse_pairs(vec![
            "--case".to_owned(),
            "selection-zh".to_owned(),
            "--confirmation".to_owned(),
            "synthetic-data-may-incur-cost".to_owned(),
        ])
        .expect("single case arguments");
        assert_eq!(
            values.get("--case").map(String::as_str),
            Some("selection-zh")
        );
    }
}
