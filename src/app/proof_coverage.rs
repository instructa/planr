use serde_json::{Value, json};

pub(crate) fn proof_status_from_coverages(scope: Value, coverages: Vec<Value>) -> Value {
    let criteria = coverages
        .into_iter()
        .map(criterion_status_from_coverage)
        .collect::<Vec<_>>();
    let pass = criteria
        .iter()
        .all(|criterion| criterion["pass"].as_bool() == Some(true));
    let actionable_gaps = criteria
        .iter()
        .flat_map(|criterion| {
            criterion["actionable_gaps"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let non_actionable_blockers = criteria
        .iter()
        .flat_map(|criterion| {
            criterion["non_actionable_blockers"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let receipts = unique_refs(&criteria, "receipt_refs");
    let attempts = unique_refs(&criteria, "attempt_refs");
    let waivers = unique_refs(&criteria, "waiver_refs");
    let suggested_next_action = criteria
        .iter()
        .find(|criterion| criterion["pass"].as_bool() != Some(true))
        .and_then(|criterion| criterion["suggested_next_action"].as_str())
        .map(|action| json!(action))
        .unwrap_or_else(|| json!("none"));
    let next_action = suggested_next_action
        .as_str()
        .filter(|action| *action != "none")
        .map(|action| json!(action))
        .unwrap_or(Value::Null);
    let actionable_now = criteria
        .iter()
        .any(|criterion| criterion["actionable_now"].as_bool() == Some(true));
    json!({
        "scope": scope,
        "active_binding": true,
        "pass": pass,
        "status": if pass { "proven" } else { "not_proven" },
        "completion_language": if pass {
            "proven by canonical Evidence coverage"
        } else {
            "not proven by canonical Evidence coverage"
        },
        "actionable_now": actionable_now,
        "actionable_gaps": actionable_gaps,
        "non_actionable_blockers": non_actionable_blockers,
        "receipts": receipts,
        "attempts": attempts,
        "waivers": waivers,
        "criteria": criteria,
        "next_action": next_action,
        "suggested_next_action": suggested_next_action,
    })
}

fn criterion_status_from_coverage(coverage: Value) -> Value {
    let mut projection = coverage["canonical_projection"].clone();
    let next_action = projection["suggested_next_action"]
        .as_str()
        .filter(|action| *action != "none")
        .map(|action| json!(action))
        .unwrap_or(Value::Null);
    projection["next_action"] = next_action;
    projection
}

fn unique_refs(criteria: &[Value], key: &str) -> Vec<Value> {
    let mut refs = Vec::<Value>::new();
    for criterion in criteria {
        for value in criterion[key].as_array().into_iter().flatten() {
            if !refs.iter().any(|existing| existing == value) {
                refs.push(value.clone());
            }
        }
    }
    refs
}
