use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Outcome, Validator};
use crate::{RetrievalUri, resources::ResourceGraph};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const SUITE_SOURCE: &str = "https://github.com/json-schema-org/JSON-Schema-Test-Suite";
const SUITE_REVISION: &str = "c0b038ad7244712cf73650f44e90d0bc5704e8c7";
const SUITE_SHA256: &str = "53e63ed0d1c1f16421623eab831e225507bf21ec12d50bdb24146fda07bf147c";
const SUITE_LICENSE_SHA256: &str =
    "837402bd25fad9b704265801ca3f92566a98157c1f9a7acd6f446299ba1c305a";
const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";
const NO_VALIDATION_DIALECT: &str =
    "http://localhost:1234/draft2020-12/metaschema-no-validation.json";
const OPTIONAL_VOCABULARY_DIALECT: &str =
    "http://localhost:1234/draft2020-12/metaschema-optional-vocabulary.json";
const FORMAT_ASSERTION_DIALECTS: [&str; 2] = [
    "http://localhost:1234/draft2020-12/format-assertion-false.json",
    "http://localhost:1234/draft2020-12/format-assertion-true.json",
];
const FIXTURE: &[u8] = include_bytes!("../../tests/fixtures/draft202012-official-suite.json");
const LICENSE: &[u8] = include_bytes!("../../tests/fixtures/JSON-Schema-Test-Suite-LICENSE");

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn embedded_fixture_preserves_the_pinned_upstream_evidence() {
    assert_eq!(hex_digest(FIXTURE), SUITE_SHA256);
    assert_eq!(hex_digest(LICENSE), SUITE_LICENSE_SHA256);

    let bundle = official_suite();
    assert_eq!(bundle["source"], SUITE_SOURCE);
    assert_eq!(bundle["revision"], SUITE_REVISION);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn mandatory_draft_2020_12_suite_matches_through_the_product_validator() {
    let bundle = official_suite();
    let files = bundle["mandatory"]
        .as_array()
        .expect("the bundle should contain mandatory suite files");
    let resources = supplied_resources(&bundle);
    let (group_count, case_count) = run_suite_files(
        files,
        &resources,
        "https://official-suite.invalid/draft2020-12",
        "mandatory",
    );

    assert_eq!(files.len(), 46, "the pinned mandatory file set changed");
    assert_eq!(group_count, 383, "the pinned mandatory group set changed");
    assert_eq!(case_count, 1_299, "the pinned mandatory case set changed");
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn claimed_optional_draft_2020_12_suite_matches_through_the_product_validator() {
    let bundle = official_suite();
    let files = bundle["optional"]
        .as_array()
        .expect("the bundle should contain claimed optional suite files");
    assert_bignum_literals(files);
    let resources = supplied_resources(&bundle);
    let (group_count, case_count) = run_suite_files(
        files,
        &resources,
        "https://official-suite.invalid/draft2020-12/optional",
        "optional",
    );

    assert_eq!(files.len(), 4, "the pinned optional file set changed");
    assert_eq!(group_count, 30, "the pinned optional group set changed");
    assert_eq!(case_count, 96, "the pinned optional case set changed");
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn official_suite() -> Value {
    serde_json::from_slice(FIXTURE).expect("the embedded official suite should be valid JSON")
}

fn assert_bignum_literals(files: &[Value]) {
    // Keep this assertion independent of fixture generation so it detects accidental loss of
    // arbitrary-precision data.
    let bignum = files
        .iter()
        .find(|file| file["file"] == "bignum.json")
        .expect("the claimed optional files should include bignum.json");
    let groups = bignum["groups"]
        .as_array()
        .expect("bignum.json should contain test groups");

    let integer = group(groups, "integer");
    assert_number(
        &integer["tests"][0]["data"],
        "12345678910111213141516171819202122232425262728293031",
    );
    assert_number(
        &integer["tests"][1]["data"],
        "-12345678910111213141516171819202122232425262728293031",
    );

    let number = group(groups, "number");
    assert_number(
        &number["tests"][0]["data"],
        "98249283749234923498293171823948729348710298301928331",
    );
    assert_number(
        &number["tests"][1]["data"],
        "-98249283749234923498293171823948729348710298301928331",
    );
    assert_number(
        &group(groups, "string")["tests"][0]["data"],
        "98249283749234923498293171823948729348710298301928331",
    );

    let maximum = group(groups, "maximum integer comparison");
    assert_number(&maximum["schema"]["maximum"], "18446744073709551615");
    assert_number(&maximum["tests"][0]["data"], "18446744073709551600");

    let positive_float = group(groups, "float comparison with high precision");
    assert_number(
        &positive_float["schema"]["exclusiveMaximum"],
        "972783798187987123879878123.18878137",
    );
    assert_number(
        &positive_float["tests"][0]["data"],
        "972783798187987123879878123.188781371",
    );

    let minimum = group(groups, "minimum integer comparison");
    assert_number(&minimum["schema"]["minimum"], "-18446744073709551615");
    assert_number(&minimum["tests"][0]["data"], "-18446744073709551600");

    let negative_float = group(
        groups,
        "float comparison with high precision on negative numbers",
    );
    assert_number(
        &negative_float["schema"]["exclusiveMinimum"],
        "-972783798187987123879878123.18878137",
    );
    assert_number(
        &negative_float["tests"][0]["data"],
        "-972783798187987123879878123.188781371",
    );
}

fn group<'a>(groups: &'a [Value], description: &str) -> &'a Value {
    groups
        .iter()
        .find(|group| group["description"] == description)
        .unwrap_or_else(|| panic!("bignum.json should contain group {description}"))
}

fn assert_number(actual: &Value, expected: &str) {
    assert!(actual.is_number(), "expected numeric literal {expected}");
    assert_eq!(
        actual.to_string(),
        expected,
        "the embedded bignum fixture changed an upstream numeric literal"
    );
}

fn supplied_resources(bundle: &Value) -> Vec<(RetrievalUri, Value)> {
    bundle["remotes"]
        .as_array()
        .expect("the bundle should contain remote resources")
        .iter()
        .filter(|resource| !FORMAT_ASSERTION_DIALECTS.contains(&required_string(resource, "uri")))
        .map(|resource| {
            let mut data_schema = resource["schema"].clone();
            add_default_dialect(&mut data_schema);
            (uri(required_string(resource, "uri")), data_schema)
        })
        .collect()
}

fn add_default_dialect(data_schema: &mut Value) {
    if let Some(schema) = data_schema.as_object_mut()
        && !schema.contains_key("$schema")
    {
        schema.insert(
            "$schema".to_owned(),
            Value::String(DRAFT_2020_12.to_owned()),
        );
    } else if data_schema.is_boolean() {
        // Resource qualification requires an explicit dialect, which a bare boolean cannot carry.
        let boolean_schema = std::mem::take(data_schema);
        *data_schema = json!({
            "$schema": DRAFT_2020_12,
            "allOf": [boolean_schema]
        });
    }
}

fn run_suite_files(
    files: &[Value],
    resources: &[(RetrievalUri, Value)],
    retrieval_base: &str,
    suite: &str,
) -> (usize, usize) {
    let mut group_count = 0;
    let mut case_count = 0;
    for file in files {
        let file_name = required_string(file, "file");
        assert!(
            !file_name.contains(['/', '\\']),
            "{suite} suite file identifier must be a relative filename: {file_name}"
        );
        for group in file["groups"]
            .as_array()
            .expect("a suite file should contain test groups")
        {
            group_count += 1;
            let description = required_string(group, "description");
            let mut group_schema = group["schema"].clone();
            add_default_dialect(&mut group_schema);
            let graph = ResourceGraph::prepare_for_validator_suite(
                uri(&format!("{retrieval_base}/{file_name}")),
                group_schema,
                resources.to_vec(),
                &[NO_VALIDATION_DIALECT, OPTIONAL_VOCABULARY_DIALECT],
            )
            .unwrap_or_else(|error| {
                panic!("failed to prepare {suite} {file_name} / {description}: {error:?}")
            });
            let validator = Validator::compile(&graph).unwrap_or_else(|error| {
                panic!("failed to compile {suite} {file_name} / {description}: {error}")
            });

            for case in group["tests"]
                .as_array()
                .expect("a suite group should contain test cases")
            {
                case_count += 1;
                let actual = match validator.validate(&case["data"]) {
                    Outcome::Valid => true,
                    Outcome::Invalid { .. } => false,
                    Outcome::Indeterminate(_) => panic!(
                        "validation was indeterminate for {suite} {file_name} / {description} / {}",
                        required_string(case, "description")
                    ),
                };
                assert_eq!(
                    actual,
                    case["valid"] == true,
                    "{suite} {file_name} / {description} / {}",
                    required_string(case, "description")
                );
            }
        }
    }

    (group_count, case_count)
}

fn uri(value: &str) -> RetrievalUri {
    RetrievalUri::parse(value).expect("official suite retrieval URIs should be absolute")
}

fn required_string<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} should be a string"))
}
