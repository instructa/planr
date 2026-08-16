use serde_json::{Value, json};
use std::env;
use std::io::Write;
use std::process::{Command, ExitCode};

const TARGET_ENV: &str = "PLANR_EVIDENCE_TARGET_JSON";
const ADAPTER_SOURCE_ARG: &str = "src/bin/planr-complete-binding-authority.rs";

struct FocusedSignal {
    signal: &'static str,
    test: &'static str,
}

const FOCUSED_SIGNALS: &[FocusedSignal] = &[
    FocusedSignal {
        signal: "plan-criteria",
        test: "complete_binding_plan_criteria_contract_rejects_invalid_identity_sets",
    },
    FocusedSignal {
        signal: "authority",
        test: "complete_binding_authority_requires_the_exact_declared_criterion_set",
    },
    FocusedSignal {
        signal: "single-owner",
        test: "binding_policy_without_obligations_holds_before_review",
    },
    FocusedSignal {
        signal: "lifecycle",
        test: "complete_binding_lifecycle_fails_closed_for_partial_active_rows",
    },
];

fn main() -> ExitCode {
    if let Err(message) = validate_adapter_args() {
        return fail(message);
    }

    let Ok(target_json) = env::var(TARGET_ENV) else {
        println!("{}", json!({ "probe": true }));
        return ExitCode::SUCCESS;
    };

    match run_target_signal(&target_json) {
        Ok(payload) => {
            println!("{payload}");
            ExitCode::SUCCESS
        }
        Err(message) => fail(message),
    }
}

fn validate_adapter_args() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(()),
        [flag, source] if flag == "--adapter-source" && source == ADAPTER_SOURCE_ARG => Ok(()),
        _ => Err(format!(
            "unsupported complete-binding adapter arguments: {}",
            args.join(" ")
        )),
    }
}

fn run_target_signal(target_json: &str) -> Result<Value, String> {
    let target = serde_json::from_str::<Value>(target_json)
        .map_err(|error| format!("{TARGET_ENV} must be JSON: {error}"))?;
    let signal = target
        .get("uri")
        .and_then(Value::as_str)
        .and_then(|uri| {
            uri.split('/')
                .filter(|segment| !segment.is_empty())
                .next_back()
        })
        .ok_or_else(|| "complete-binding target uri must end with a signal".to_string())?;
    let focused = FOCUSED_SIGNALS
        .iter()
        .find(|candidate| candidate.signal == signal)
        .ok_or_else(|| format!("unsupported focused verification signal: {signal}"))?;

    let output = Command::new("cargo")
        .args([
            "test",
            "--test",
            "e2e",
            focused.test,
            "--",
            "--exact",
            "--test-threads=1",
        ])
        .env_remove("PLANR_EVIDENCE_TARGET_JSON")
        .env_remove("PLANR_EVIDENCE_ENVIRONMENT_JSON")
        .env_remove("PLANR_EVIDENCE_EXECUTION_CONTRACT_DIGEST")
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .map_err(|error| error.to_string())?;

    if !output.status.success() {
        return Err(if output.stderr.is_empty() {
            tail_excerpt(&output.stdout, 4096)
        } else {
            tail_excerpt(&output.stderr, 4096)
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout
        .lines()
        .any(|line| line.starts_with("test result: ok. 1 passed; 0 failed;"))
    {
        return Err(format!(
            "focused verification must execute exactly one passing test: {}",
            tail_excerpt(&output.stdout, 4096)
        ));
    }

    Ok(json!({
        "status": "passed",
        "verification_mode": "no_model",
        "signal": signal,
        "checks": [format!("e2e:{}", focused.test)],
    }))
}

fn tail_excerpt(bytes: &[u8], limit: usize) -> String {
    let text = String::from_utf8_lossy(bytes);
    let chars = text.chars().collect::<Vec<_>>();
    let start = chars.len().saturating_sub(limit);
    chars[start..].iter().collect()
}

fn fail(message: String) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "{}", message.trim_end());
    ExitCode::FAILURE
}
