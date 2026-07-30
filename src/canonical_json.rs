use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub(crate) fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>> {
    serde_jcs::to_vec(value).context("canonicalizing JSON value with RFC 8785/JCS")
}

pub(crate) fn sha256_prefixed_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(crate) fn sha256_json_digest(value: &Value) -> Result<String> {
    Ok(sha256_prefixed_bytes(&canonical_json_bytes(value)?))
}

pub(crate) fn sha256_json_digest_without_top_level_field(
    value: &Value,
    field: &str,
) -> Result<String> {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove(field);
    }
    sha256_json_digest(&value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::{fs, path::Path};

    fn json_fences(markdown: &str) -> Vec<String> {
        let mut fences = Vec::new();
        let mut current = Vec::new();
        let mut in_json = false;

        for line in markdown.lines() {
            if line.trim() == "```json" {
                in_json = true;
                current.clear();
                continue;
            }
            if in_json && line.trim() == "```" {
                in_json = false;
                fences.push(current.join("\n"));
                continue;
            }
            if in_json {
                current.push(line);
            }
        }
        fences
    }

    #[test]
    fn eval_contract_digest_vectors_use_production_jcs() {
        let contract = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/contracts/EVAL_CONTRACT_V1.md"),
        )
        .expect("Eval contract is checked in");
        let values = json_fences(&contract)
            .into_iter()
            .map(|fence| serde_json::from_str::<Value>(&fence).expect("contract JSON parses"))
            .collect::<Vec<_>>();
        let vectors = values
            .iter()
            .find_map(|value| {
                value
                    .pointer("/object/canonicalization_vectors")
                    .and_then(Value::as_array)
            })
            .expect("contract has canonicalization vectors");

        for vector in vectors {
            let input = serde_json::from_str::<Value>(vector["input_json"].as_str().unwrap())
                .expect("vector input parses");
            let actual =
                String::from_utf8(canonical_json_bytes(&input).expect("vector canonicalizes"))
                    .expect("canonical JSON is UTF-8");
            assert_eq!(actual, vector["canonical_json"].as_str().unwrap());
            assert_eq!(
                sha256_prefixed_bytes(actual.as_bytes()),
                format!("sha256:{}", vector["sha256"].as_str().unwrap())
            );
        }
    }

    #[test]
    fn evidence_contract_digest_vectors_use_production_jcs() {
        for (path, digest_field) in [
            (
                "docs/contracts/fixtures/evidence/v1/examples/evidence-receipt.json",
                "receipt_digest",
            ),
            (
                "docs/contracts/fixtures/evidence/v1/examples/evidence-policy.json",
                "policy_digest",
            ),
        ] {
            let value = serde_json::from_str::<Value>(
                &fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
                    .expect("Evidence fixture is checked in"),
            )
            .expect("Evidence fixture parses");
            let expected = value[digest_field].as_str().unwrap();
            let actual = sha256_json_digest_without_top_level_field(&value, digest_field)
                .expect("Evidence fixture canonicalizes");
            assert_eq!(actual, expected, "{path} {digest_field} drifted");
        }
    }
}
