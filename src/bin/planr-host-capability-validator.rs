use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::Cursor;
use std::io::{self, Read};
use std::path::PathBuf;

const IDENTITY_SCHEMA_VERSION: &str = "planr.host_capability_validator_identity.v1";
const RESULT_SCHEMA_VERSION: &str = "planr.host_capability_validator_result.v1";
const VALIDATOR_NAME: &str = "planr-host-capability-validator";
const VALIDATOR_VERSION: &str = "1.0.0";

fn main() {
    let command = match read_command() {
        Ok(command) => command,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let input = match command {
        ValidatorCommand::Identity => {
            println!(
                "{}",
                serde_json::json!({
                    "schema_version": IDENTITY_SCHEMA_VERSION,
                    "validator": VALIDATOR_NAME,
                    "validator_version": VALIDATOR_VERSION
                })
            );
            return;
        }
        ValidatorCommand::ValidateScreenshot(path) => match validate_screenshot_file(&path) {
            Ok(digest) => {
                println!(
                    "{}",
                    serde_json::json!({
                        "schema_version": RESULT_SCHEMA_VERSION,
                        "validator": VALIDATOR_NAME,
                        "validator_version": VALIDATOR_VERSION,
                        "verdict": "pass",
                        "input_digest": digest,
                        "validated_raw_documents": 0,
                        "validated_instances": 0
                    })
                );
                return;
            }
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        },
        ValidatorCommand::Validate(input) => input,
    };
    let value: Value = match serde_json::from_str(&input) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("input must be JSON: {error}");
            std::process::exit(1);
        }
    };
    let input_digest = sha256_prefixed(input.as_bytes());
    run(value, input_digest);
}

enum ValidatorCommand {
    Identity,
    ValidateScreenshot(PathBuf),
    Validate(String),
}

fn read_command() -> Result<ValidatorCommand, String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        let mut input = String::new();
        io::stdin()
            .read_to_string(&mut input)
            .map_err(|error| format!("failed to read stdin: {error}"))?;
        return Ok(ValidatorCommand::Validate(input));
    }
    if args.len() == 1 && args[0] == "--identity" {
        return Ok(ValidatorCommand::Identity);
    }
    if args.len() == 2 && args[0] == "--input" {
        return fs::read_to_string(&args[1])
            .map(ValidatorCommand::Validate)
            .map_err(|error| format!("failed to read input file {}: {error}", args[1]));
    }
    if args.len() == 2 && args[0] == "--validate-screenshot" {
        return Ok(ValidatorCommand::ValidateScreenshot(PathBuf::from(
            &args[1],
        )));
    }
    Err(
        "usage: planr-host-capability-validator [--identity|--input PATH|--validate-screenshot PATH]"
            .to_string(),
    )
}

fn sha256_prefixed(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn validate_screenshot_file(path: &PathBuf) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "screenshot image must be readable {}: {error}",
            path.display()
        )
    })?;
    let digest = sha256_prefixed(&bytes);
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|error| format!("screenshot image format detection failed: {error}"))?;
    let format = reader
        .format()
        .ok_or_else(|| "screenshot image format must be PNG, JPEG, or WebP".to_string())?;
    if !matches!(
        format,
        image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::WebP
    ) {
        return Err("screenshot image format must be PNG, JPEG, or WebP".to_string());
    }
    let image = reader
        .decode()
        .map_err(|error| format!("screenshot image must decode completely: {error}"))?;
    if image.width() == 0 || image.height() == 0 {
        return Err("screenshot image dimensions must be non-zero".to_string());
    }
    Ok(digest)
}

fn run(value: Value, input_digest: String) {
    let result = validate_input(value, &input_digest);
    match result {
        Ok(summary) => println!("{summary}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn validate_input(value: Value, input_digest: &str) -> Result<Value, String> {
    match serde_json::from_value::<ValidatorInput>(value) {
        Ok(ValidatorInput::Instances(instances)) => {
            validate_capability_instances(&instances).map(|()| {
                serde_json::json!({
                    "schema_version": RESULT_SCHEMA_VERSION,
                    "validator": VALIDATOR_NAME,
                    "validator_version": VALIDATOR_VERSION,
                    "verdict": "pass",
                    "input_digest": input_digest,
                    "validated_instances": instances.len()
                })
            })
        }
        Ok(ValidatorInput::Bundle(bundle)) => validate_bundle(&bundle).map(|()| {
            serde_json::json!({
                "schema_version": RESULT_SCHEMA_VERSION,
                "validator": VALIDATOR_NAME,
                "validator_version": VALIDATOR_VERSION,
                "verdict": "pass",
                "input_digest": input_digest,
                "validated_raw_documents": bundle.raw_documents.len(),
                "validated_instances": bundle.capability_instances.len()
            })
        }),
        Err(error) => Err(format!(
            "input must be a JSON array of capability instances or validation bundle: {error}"
        )),
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ValidatorInput {
    Bundle(ValidationBundle),
    Instances(Vec<Value>),
}

#[derive(Deserialize)]
struct ValidationBundle {
    raw_documents: Vec<Value>,
    expected_document: Value,
    provenance_document: Value,
    schemas: SchemaBundle,
    capability_instances: Vec<Value>,
}

#[derive(Deserialize)]
struct SchemaBundle {
    raw: Value,
    expected: Value,
    provenance: Value,
}

fn validate_bundle(bundle: &ValidationBundle) -> Result<(), String> {
    validate_contract_schema(&bundle.schemas.raw, "schemas.raw")?;
    validate_contract_schema(&bundle.schemas.expected, "schemas.expected")?;
    validate_contract_schema(&bundle.schemas.provenance, "schemas.provenance")?;
    for (index, raw) in bundle.raw_documents.iter().enumerate() {
        validate_json_schema_instance(
            &bundle.schemas.raw,
            raw,
            &format!("raw_documents[{index}]"),
        )?;
    }
    validate_json_schema_instance(
        &bundle.schemas.expected,
        &bundle.expected_document,
        "expected_document",
    )?;
    validate_json_schema_instance(
        &bundle.schemas.provenance,
        &bundle.provenance_document,
        "provenance_document",
    )?;
    validate_capability_instances(&bundle.capability_instances)
}

fn validate_contract_schema(schema: &Value, label: &str) -> Result<(), String> {
    let object = schema
        .as_object()
        .ok_or_else(|| format!("{label} must be a JSON Schema object"))?;
    let properties = object
        .get("properties")
        .and_then(Value::as_object)
        .ok_or_else(|| format!("{label} must not be a permissive empty schema"))?;
    if properties.is_empty() {
        return Err(format!("{label} must not be a permissive empty schema"));
    }
    if !object.contains_key("$schema") {
        return Err(format!("{label} must declare $schema"));
    }
    if object.get("additionalProperties") != Some(&Value::Bool(false)) {
        return Err(format!(
            "{label} root object must set additionalProperties false"
        ));
    }
    if object
        .get("required")
        .and_then(Value::as_array)
        .is_none_or(Vec::is_empty)
    {
        return Err(format!("{label} root object must declare required fields"));
    }
    validate_schema_strictness(schema, label)
}

fn validate_schema_strictness(schema: &Value, label: &str) -> Result<(), String> {
    match schema {
        Value::Object(object) => {
            if object.contains_key("properties")
                && object.get("additionalProperties") != Some(&Value::Bool(false))
            {
                return Err(format!(
                    "{label} object schemas with fixed properties must set additionalProperties false"
                ));
            }
            for (key, value) in object {
                validate_schema_strictness(value, &format!("{label}.{key}"))?;
            }
        }
        Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                validate_schema_strictness(value, &format!("{label}[{index}]"))?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_json_schema_instance(
    schema: &Value,
    instance: &Value,
    label: &str,
) -> Result<(), String> {
    let validator = jsonschema::draft202012::options()
        .build(schema)
        .map_err(|error| format!("{label} schema failed to compile: {error}"))?;
    let errors = validator
        .iter_errors(instance)
        .map(|error| format!("{}: {}", error.instance_path(), error))
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{label} failed Draft 2020-12 validation: {}",
            errors.join("; ")
        ))
    }
}

fn validate_capability_instances(instances: &[Value]) -> Result<(), String> {
    for (index, instance) in instances.iter().enumerate() {
        planr::evidence::parse_verification_capability_instance(instance.clone()).map_err(
            |error| {
                format!("capability_instances[{index}] failed canonical Evidence parse: {error}")
            },
        )?;
    }
    Ok(())
}
