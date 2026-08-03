use schemaform::{
    CompileError, Dialect, FormDefinition, QualificationError, RetrievalUri, SchemaResource,
    SubmissionOutcome,
    form::{SubmissionBlocker, ValidationOutcomeView},
};
use serde_json::{Value, json};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";
const WORKSPACE_MANIFEST: &str = include_str!("../../../Cargo.toml");
const WORKSPACE_LOCK: &str = include_str!("../../../Cargo.lock");
const CORE_MANIFEST: &str = include_str!("../Cargo.toml");
const DIOXUS_MANIFEST: &str = include_str!("../../schemaform-dioxus/Cargo.toml");

#[derive(Clone, Copy, Debug)]
enum ExpectedOutcome {
    Valid,
    Invalid,
}

fn uri(value: &str) -> RetrievalUri {
    RetrievalUri::parse(value).expect("the fixture URI should be absolute and fragment-free")
}

fn assert_outcomes(property_data_schema: Value, cases: &[(Value, ExpectedOutcome)]) {
    let definition = FormDefinition::compile(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": { "value": property_data_schema }
    }))
    .expect("the validator configuration fixture should compile");

    assert_definition_outcomes(&definition, cases);
}

fn assert_definition_outcomes(definition: &FormDefinition, cases: &[(Value, ExpectedOutcome)]) {
    for (value, expected) in cases {
        let form = definition
            .create_form(json!({ "value": value }))
            .expect("the validator configuration fixture should create a form");
        let view = form.view();
        let actual = view.validation_outcome();
        assert!(
            matches!(
                (actual, expected),
                (ValidationOutcomeView::Valid, ExpectedOutcome::Valid)
                    | (
                        ValidationOutcomeView::Invalid { .. },
                        ExpectedOutcome::Invalid
                    )
            ),
            "unexpected outcome {actual:?} for {value}"
        );
    }
}

fn assert_form_data_valid(definition: &FormDefinition, cases: &[Value]) {
    for form_data in cases {
        let form = definition
            .create_form(form_data.clone())
            .expect("schema-invalid data should remain constructible");
        assert_eq!(
            form.view().validation_outcome(),
            ValidationOutcomeView::Valid,
            "unexpected invalid outcome for {form_data}"
        );
    }
}

fn locked_package(package: &str) -> Option<&str> {
    let mut entries = WORKSPACE_LOCK.split("[[package]]").filter(|entry| {
        entry
            .lines()
            .any(|line| line == format!("name = \"{package}\""))
    });
    let entry = entries.next()?;
    assert!(
        entries.next().is_none(),
        "the qualified lock must contain exactly one {package} version"
    );
    Some(entry)
}

fn locked_version(package: &str) -> Option<&str> {
    locked_package(package)?
        .lines()
        .find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
}

#[test]
fn dependency_and_validator_configuration_match_the_qualified_release() {
    assert!(WORKSPACE_MANIFEST.contains(
        "jsonschema = { version = \"=0.47.0\", default-features = false, features = [\"arbitrary-precision\"] }"
    ));
    assert!(WORKSPACE_MANIFEST.contains("referencing = \"=0.47.0\""));
    assert!(
        WORKSPACE_MANIFEST.contains(
            "serde_json = { version = \"=1.0.150\", features = [\"arbitrary_precision\"] }"
        )
    );
    for (package, version) in [
        ("jsonschema", "0.47.0"),
        ("jsonschema-regex", "0.47.0"),
        ("referencing", "0.47.0"),
        ("serde_json", "1.0.150"),
        ("fancy-regex", "0.18.0"),
        ("regex", "1.13.1"),
    ] {
        assert_eq!(
            locked_version(package),
            Some(version),
            "unexpected {package} pin"
        );
    }
    let jsonschema = locked_package("jsonschema").expect("jsonschema should be locked");
    for dependency in [
        "fancy-regex",
        "jsonschema-regex",
        "referencing",
        "regex",
        "serde_json",
    ] {
        assert!(
            jsonschema.contains(&format!(" \"{dependency}\",")),
            "jsonschema should resolve through the qualified {dependency} pin"
        );
    }
}

#[test]
fn public_packages_do_not_expose_validation_fault_features() {
    for manifest in [CORE_MANIFEST, DIOXUS_MANIFEST] {
        assert!(!manifest.contains("test-validation-faults"));
        assert!(!manifest.contains("[features]"));
    }
    // The adapter inherits schemaform from the workspace, which pins it by path and
    // version. Matching both lines exactly keeps either side from quietly enabling
    // features on the dependency.
    assert!(DIOXUS_MANIFEST.contains("schemaform = { workspace = true }"));
    assert!(
        WORKSPACE_MANIFEST
            .contains("schemaform = { path = \"crates/schemaform\", version = \"0.0.0\" }")
    );
}

#[cfg(not(schemaform_test_validation_faults))]
#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn validation_fault_annotation_is_inert_in_production_builds() {
    let definition = FormDefinition::compile(json!({
        "$schema": DRAFT_2020_12,
        "x-schemaform-test-validation-fault": true,
        "type": "object",
        "additionalProperties": false,
        "required": ["value"],
        "properties": {
            "value": { "type": "string", "minLength": 3 }
        }
    }))
    .expect("unknown annotations remain valid trusted data-schema annotations");
    let mut form = definition
        .create_form(json!({ "value": "" }))
        .expect("the invalid form data should remain constructible");

    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, truncated: false }
            if findings.len() == 1 && findings[0].code() == "minLength"
    ));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().all(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
#[test]
fn resolved_validator_features_disable_defaults_and_enable_arbitrary_precision() {
    let workspace_manifest = format!("{}/../../Cargo.toml", env!("CARGO_MANIFEST_DIR"));
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "tree",
            "--locked",
            "--manifest-path",
            &workspace_manifest,
            "-e",
            "features",
            "-i",
            "jsonschema@0.47.0",
        ])
        .output()
        .expect("cargo tree should inspect the resolved validator feature graph");
    assert!(output.status.success(), "cargo tree should succeed");
    let features = String::from_utf8(output.stdout).expect("cargo tree output should be UTF-8");

    assert!(features.contains("jsonschema feature \"arbitrary-precision\""));
    for disabled in [
        "jsonschema feature \"default\"",
        "jsonschema feature \"resolve-file\"",
        "jsonschema feature \"resolve-http\"",
        "jsonschema feature \"tls-aws-lc-rs\"",
    ] {
        assert!(
            !features.contains(disabled),
            "unexpected feature: {disabled}"
        );
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn formats_regex_and_arbitrary_numbers_match_the_qualified_validator() {
    assert_outcomes(
        json!({ "type": "string", "format": "email" }),
        &[(json!("not-an-email"), ExpectedOutcome::Valid)],
    );
    assert_outcomes(
        json!({ "type": "string", "format": "unknown-format" }),
        &[(json!("anything"), ExpectedOutcome::Valid)],
    );
    assert_outcomes(
        json!({ "type": "string", "pattern": "^[a-z]+$" }),
        &[
            (json!("browser"), ExpectedOutcome::Valid),
            (json!("browser-123"), ExpectedOutcome::Invalid),
        ],
    );

    let integer_minimum = serde_json::from_str::<Value>("184467440737095516160")
        .expect("the arbitrary-precision integer minimum should parse");
    let integer_valid = serde_json::from_str::<Value>("184467440737095516161")
        .expect("the arbitrary-precision valid integer should parse");
    let integer_invalid = serde_json::from_str::<Value>("184467440737095516159")
        .expect("the arbitrary-precision invalid integer should parse");
    assert_outcomes(
        json!({ "type": "integer", "minimum": integer_minimum }),
        &[
            (integer_valid, ExpectedOutcome::Valid),
            (integer_invalid, ExpectedOutcome::Invalid),
        ],
    );

    let decimal_data_schema = serde_json::from_str::<Value>(
        r#"{"type":"number","minimum":0.1000000000000000000000000000000000000001}"#,
    )
    .expect("the arbitrary-precision decimal schema should parse");
    let decimal_valid =
        serde_json::from_str::<Value>("0.10000000000000000000000000000000000000011")
            .expect("the arbitrary-precision valid decimal should parse");
    let decimal_invalid =
        serde_json::from_str::<Value>("0.10000000000000000000000000000000000000009")
            .expect("the arbitrary-precision invalid decimal should parse");
    assert_outcomes(
        decimal_data_schema,
        &[
            (decimal_valid, ExpectedOutcome::Valid),
            (decimal_invalid, ExpectedOutcome::Invalid),
        ],
    );

    let unsupported_pattern = FormDefinition::compile(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": { "value": { "type": "string", "pattern": "(a)\\1" } }
    }))
    .err()
    .expect("a pattern unsupported by the qualified regex engine should fail");
    assert!(matches!(
        unsupported_pattern,
        CompileError::Qualification(QualificationError::InvalidSchema { .. })
    ));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn normalized_findings_are_deduplicated_ordered_and_do_not_leak_failing_values() {
    let definition = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "$id": "https://schemas.example/finding-contract.json",
        "type": "object",
        "properties": {
            "value": {
                "allOf": [
                    { "$ref": "#/$defs/text" },
                    { "$ref": "#/$defs/text" }
                ]
            }
        },
        "$defs": {
            "text": {
                "type": "string",
                "maxLength": 5,
                "minLength": 20,
                "pattern": "^[A-Z]+$"
            }
        }
    }))
    .analyze()
    .expect("the finding contract fixture should remain analyzable")
    .into_parts()
    .0;
    let secret = "secretvalue";
    let form = definition
        .create_form(json!({ "value": secret }))
        .expect("invalid form data should remain constructible");
    let view = form.view();
    let ValidationOutcomeView::Invalid {
        findings,
        truncated: false,
    } = view.validation_outcome()
    else {
        panic!("the finding contract fixture should be invalid without truncation");
    };

    assert_eq!(findings.len(), 3, "duplicate references must collapse");
    assert_eq!(
        findings
            .iter()
            .map(|finding| (
                finding.code(),
                finding.instance_location().as_str(),
                finding.keyword_location().resource().as_str(),
                finding.keyword_location().pointer().as_str(),
                finding.parameters().clone(),
            ))
            .collect::<Vec<_>>(),
        [
            (
                "maxLength",
                "/value",
                "https://schemas.example/finding-contract.json",
                "/$defs/text/maxLength",
                json!({ "limit": 5 }),
            ),
            (
                "minLength",
                "/value",
                "https://schemas.example/finding-contract.json",
                "/$defs/text/minLength",
                json!({ "limit": 20 }),
            ),
            (
                "pattern",
                "/value",
                "https://schemas.example/finding-contract.json",
                "/$defs/text/pattern",
                json!({ "pattern": "^[A-Z]+$" }),
            ),
        ]
    );
    assert!(!format!("{findings:?}").contains(secret));

    let expected = findings.to_vec();
    let mut form = form;
    let preparation = form.prepare_submission();
    let SubmissionOutcome::Blocked(blockers) = preparation.outcome() else {
        panic!("normalized validation findings should block submission");
    };
    assert_eq!(
        blockers
            .iter()
            .filter_map(|blocker| match blocker {
                SubmissionBlocker::Validation(finding) => Some(finding.clone()),
                _ => None,
            })
            .collect::<Vec<_>>(),
        expected
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn only_the_standard_draft_2020_12_dialect_is_selected() {
    FormDefinition::compile(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "additionalProperties": false,
        "properties": { "value": { "type": "string" } }
    }))
    .expect("the canonical standard Draft 2020-12 dialect should compile");
    FormDefinition::compiler(json!({
        "type": "object",
        "additionalProperties": false,
        "properties": { "value": { "type": "string" } }
    }))
    .default_dialect(Dialect::Draft202012)
    .compile()
    .expect("the explicit standard Draft 2020-12 default should compile");

    for dialect in [
        "http://json-schema.org/draft/2020-12/schema",
        "HTTPS://json-schema.org/draft/2020-12/schema",
        "https://JSON-SCHEMA.ORG/draft/2020-12/schema",
        "https://json-schema.org/draft/2020-12/schema##",
    ] {
        let error = FormDefinition::compile(json!({
            "$schema": dialect,
            "type": "object"
        }))
        .err()
        .unwrap_or_else(|| panic!("noncanonical dialect spelling {dialect} should fail"));
        assert!(matches!(
            error,
            CompileError::Qualification(QualificationError::UnsupportedDialect { .. })
        ));
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn legacy_dependencies_remains_an_annotation_under_draft_2020_12() {
    let definition = FormDefinition::compile(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": { "foo": { "type": "integer" } },
        "dependencies": { "foo": ["bar"] }
    }))
    .expect("the legacy keyword should remain an annotation");
    let form = definition
        .create_form(json!({ "foo": 1 }))
        .expect("form data without the legacy dependency should be accepted");
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );

    let additional_items = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": {
            "value": { "type": "array", "items": false, "additionalItems": false }
        }
    }))
    .analyze()
    .expect("the legacy keyword should remain an annotation");
    assert_definition_outcomes(
        additional_items.definition(),
        &[
            (json!([]), ExpectedOutcome::Valid),
            (json!([1]), ExpectedOutcome::Invalid),
        ],
    );

    let distinguishing_additional_items = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": {
            "value": { "type": "array", "additionalItems": false }
        }
    }))
    .analyze()
    .expect("additionalItems alone should remain an annotation");
    assert_definition_outcomes(
        distinguishing_additional_items.definition(),
        &[(json!([1]), ExpectedOutcome::Valid)],
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn legacy_keywords_remain_annotations_in_local_opaque_reference_targets() {
    let definition = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": {
            "dependency": { "$ref": "#/x-opaque/dependency" },
            "additional": { "$ref": "#/x-opaque/additional" },
            "recursive": { "$ref": "#/x-opaque/recursive" }
        },
        "$defs": { "reject": false },
        "x-opaque": {
            "dependency": {
                "type": "object",
                "dependencies": { "foo": ["bar"] }
            },
            "additional": {
                "type": "array",
                "additionalItems": false
            },
            "recursive": { "$recursiveRef": "#/$defs/reject" }
        }
    }))
    .analyze()
    .expect("opaque reference targets with legacy annotations should analyze")
    .into_parts()
    .0;

    assert_form_data_valid(
        &definition,
        &[
            json!({ "dependency": { "foo": 1 } }),
            json!({ "additional": [1] }),
            json!({ "recursive": "accepted" }),
        ],
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn legacy_keywords_remain_annotations_in_caller_opaque_reference_targets() {
    let caller_uri = "https://schemas.example/legacy-targets.json";
    let definition = FormDefinition::compiler(json!({
        "$schema": DRAFT_2020_12,
        "type": "object",
        "properties": {
            "dependency": { "$ref": format!("{caller_uri}#/x-opaque/dependency") },
            "additional": { "$ref": format!("{caller_uri}#/x-opaque/additional") },
            "recursive": { "$ref": format!("{caller_uri}#/x-opaque/recursive") }
        }
    }))
    .resource(SchemaResource::new(
        uri(caller_uri),
        json!({
            "$schema": DRAFT_2020_12,
            "$defs": { "reject": false },
            "x-opaque": {
                "dependency": {
                    "type": "object",
                    "dependencies": { "foo": ["bar"] }
                },
                "additional": {
                    "type": "array",
                    "additionalItems": false
                },
                "recursive": { "$recursiveRef": "#/$defs/reject" }
            }
        }),
    ))
    .analyze()
    .expect("caller opaque reference targets with legacy annotations should analyze")
    .into_parts()
    .0;

    assert_form_data_valid(
        &definition,
        &[
            json!({ "dependency": { "foo": 1 } }),
            json!({ "additional": [1] }),
            json!({ "recursive": "accepted" }),
        ],
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn complete_recursive_resources_construct_without_implicit_retrieval() {
    let root = json!({
        "$schema": DRAFT_2020_12,
        "$id": "https://schemas.example/root.json",
        "type": "object",
        "properties": {
            "value": { "$ref": "child.json" }
        }
    });
    let compiler =
        FormDefinition::compiler(root).root_uri(uri("https://retrieval.example/root.json"));

    let missing = compiler
        .clone()
        .compile()
        .err()
        .expect("an incomplete graph should fail instead of retrieving a resource");
    assert!(matches!(
        missing,
        CompileError::Qualification(QualificationError::UnresolvedReference { .. })
    ));

    let definition = compiler
        .resource(SchemaResource::new(
            uri("https://retrieval.example/child.json"),
            json!({
                "$schema": DRAFT_2020_12,
                "$id": "https://schemas.example/child.json",
                "type": "string",
                "$defs": {
                    "root": { "$ref": "root.json" }
                }
            }),
        ))
        .compile()
        .expect("the complete recursive graph should prepare and construct a validator");

    assert_definition_outcomes(
        &definition,
        &[
            (json!("valid"), ExpectedOutcome::Valid),
            (json!(1), ExpectedOutcome::Invalid),
        ],
    );
}
