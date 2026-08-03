use schemaform::{
    CompilationProfile, FormBuildError, FormDataLimits, FormDefinition, JsonParseError,
    JsonPointer, ResourceLimitError, ResourceLimitPhase,
    json::{parse_form_data, parse_ui_schema_v1},
};
use serde_json::{Value, json};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

fn object_definition(properties: Value) -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": DIALECT,
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
    }))
    .expect("the qualification schema should compile")
}

fn resource_limit(error: FormBuildError) -> ResourceLimitError {
    let FormBuildError::ResourceLimit(error) = error else {
        panic!("expected a form resource limit, got {error:?}");
    };
    error
}

fn parsed_resource_limit(error: JsonParseError) -> ResourceLimitError {
    let JsonParseError::ResourceLimit(error) = error else {
        panic!("expected a parsed-input resource limit, got {error:?}");
    };
    error
}

fn assert_limit(
    error: ResourceLimitError,
    dimension: &str,
    maximum: usize,
    observed: usize,
    path: &str,
) {
    assert_eq!(error.phase(), ResourceLimitPhase::Construction);
    assert_eq!(error.dimension(), dimension);
    assert_eq!(error.maximum(), maximum);
    assert_eq!(error.observed(), observed);
    assert_eq!(error.path(), &JsonPointer::parse(path).unwrap());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn parsed_form_data_source_limits_have_below_exact_and_above_boundaries() {
    let bytes = br#"{"name":"Ada"}"#;
    for maximum in [bytes.len() - 1, bytes.len(), bytes.len() + 1] {
        let result = parse_form_data(bytes, &FormDataLimits::default().max_bytes(maximum));
        assert_eq!(result.is_ok(), maximum >= bytes.len());
        if maximum < bytes.len() {
            assert_limit(
                parsed_resource_limit(result.unwrap_err()),
                "bytes",
                maximum,
                bytes.len(),
                "",
            );
        }
    }

    // Object, one member name, and one scalar value are three semantic tokens.
    for maximum in [2, 3, 4] {
        let result = parse_form_data(bytes, &FormDataLimits::default().max_tokens(maximum));
        assert_eq!(result.is_ok(), maximum >= 3);
        if maximum < 3 {
            assert_limit(
                parsed_resource_limit(result.unwrap_err()),
                "tokens",
                maximum,
                3,
                "",
            );
        }
    }

    let deep = format!("{}null{}", "[".repeat(20), "]".repeat(20));
    assert_limit(
        parsed_resource_limit(
            parse_form_data(deep.as_bytes(), &FormDataLimits::default().max_depth(19)).unwrap_err(),
        ),
        "depth",
        19,
        20,
        "",
    );
    assert!(parse_form_data(deep.as_bytes(), &FormDataLimits::default().max_depth(20)).is_ok());
    assert!(parse_form_data(deep.as_bytes(), &FormDataLimits::default().max_depth(21)).is_ok());

    let escaped = br#"{"name":"\u00e9"}"#;
    assert_limit(
        parsed_resource_limit(
            parse_form_data(escaped, &FormDataLimits::default().max_scalar_bytes(3)).unwrap_err(),
        ),
        "scalar_bytes",
        3,
        4,
        "",
    );
    assert!(
        parse_form_data(escaped, &FormDataLimits::default().max_scalar_bytes(4)).is_ok(),
        "source preflight measures decoded strings rather than escape spelling"
    );
    assert!(parse_form_data(escaped, &FormDataLimits::default().max_scalar_bytes(5)).is_ok());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn parsed_and_pre_parsed_form_data_enforce_post_parse_boundaries() {
    let bytes = br#"{"items":["one","two"]}"#;
    let cases = [
        ("depth", 2usize),
        ("nodes", 4),
        ("members", 1),
        ("collection_length", 2),
        ("scalar_bytes", 5),
    ];

    for (dimension, observed) in cases {
        for maximum in [observed - 1, observed, observed + 1] {
            let limits = match dimension {
                "depth" => FormDataLimits::default().max_depth(maximum),
                "nodes" => FormDataLimits::default().max_nodes(maximum),
                "members" => FormDataLimits::default().max_members(maximum),
                "collection_length" => FormDataLimits::default().max_collection_length(maximum),
                "scalar_bytes" => FormDataLimits::default().max_scalar_bytes(maximum),
                _ => unreachable!(),
            };
            let parsed = parse_form_data(bytes, &limits);
            assert_eq!(parsed.is_ok(), maximum >= observed);
            if maximum < observed {
                assert_limit(
                    parsed_resource_limit(parsed.unwrap_err()),
                    dimension,
                    maximum,
                    observed,
                    "",
                );
            }

            let result = object_definition(json!({}))
                .form(json!({
                    "items": ["one", "two"]
                }))
                .limits(limits)
                .build();
            if maximum < observed {
                let error = match result {
                    Err(error) => resource_limit(error),
                    Ok(_) => panic!("the above-limit value must fail"),
                };
                assert_eq!(error.dimension(), dimension);
                assert_eq!(error.maximum(), maximum);
                assert_eq!(error.observed(), observed);
            } else {
                result.expect("exact and below-limit values should construct");
            }
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn ui_schema_source_and_post_parse_limits_have_boundaries() {
    let bytes = br#"{"version":1,"root":{"type":"stack","value":{"children":[]}}}"#;
    for maximum in [bytes.len() - 1, bytes.len(), bytes.len() + 1] {
        let result = parse_ui_schema_v1(
            bytes,
            &CompilationProfile::default().max_ui_schema_bytes(maximum),
        );
        assert_eq!(result.is_ok(), maximum >= bytes.len());
        if maximum < bytes.len() {
            assert_limit(
                parsed_resource_limit(result.unwrap_err()),
                "bytes",
                maximum,
                bytes.len(),
                "",
            );
        }
    }

    for maximum in [10, 11, 12] {
        let result = parse_ui_schema_v1(
            bytes,
            &CompilationProfile::default().max_ui_schema_tokens(maximum),
        );
        assert_eq!(result.is_ok(), maximum >= 11);
        if maximum < 11 {
            assert_limit(
                parsed_resource_limit(result.unwrap_err()),
                "tokens",
                maximum,
                11,
                "",
            );
        }
    }

    let cases = [
        ("depth", 3usize),
        ("nodes", 9),
        ("members", 8),
        ("collection", 3),
        ("scalar", 19),
    ];
    for (dimension, observed) in cases {
        for maximum in [observed - 1, observed, observed + 1] {
            let profile = match dimension {
                "depth" => CompilationProfile::default().max_ui_schema_depth(maximum),
                "nodes" => CompilationProfile::default().max_ui_schema_nodes(maximum),
                "members" => CompilationProfile::default().max_ui_schema_members(maximum),
                "collection" => {
                    CompilationProfile::default().max_ui_schema_collection_length(maximum)
                }
                "scalar" => CompilationProfile::default().max_ui_schema_scalar_bytes(maximum),
                _ => unreachable!(),
            };
            let parsed = parse_ui_schema_v1(bytes, &profile);
            assert_eq!(
                parsed.is_ok(),
                maximum >= observed,
                "{dimension} at {maximum}"
            );
            if maximum < observed {
                let exact_dimension = match dimension {
                    "collection" => "collection_length",
                    "scalar" => "scalar_bytes",
                    other => other,
                };
                assert_limit(
                    parsed_resource_limit(parsed.unwrap_err()),
                    exact_dimension,
                    maximum,
                    observed,
                    "",
                );
            }
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn pre_parsed_inputs_enforce_only_post_parse_limits() {
    let namespace = "https://example.com/x";
    let ui_schema: schemaform::ui::v1::UiSchema = serde_json::from_str(&format!(
        r#"{{
            "version": 1,
            "root": {{
                "type": "stack",
                "value": {{
                    "extensions": {{"{namespace}": {{"key": ["long"]}}}},
                    "children": [{{
                        "type": "text",
                        "value": {{"content": {{"fallback": "Hello"}}}}
                    }}]
                }}
            }}
        }}"#
    ))
    .unwrap();
    let data_schema = json!({
        "$schema": DIALECT,
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    });

    FormDefinition::compiler(data_schema.clone())
        .ui_schema(ui_schema.clone())
        .profile(
            CompilationProfile::default()
                .max_ui_schema_bytes(0)
                .max_ui_schema_tokens(0),
        )
        .compile()
        .expect("pre-parsed UI schemas cannot be charged for source parsing");
    type Configure = fn(CompilationProfile, usize) -> CompilationProfile;
    for (observed, configure) in [
        (7usize, CompilationProfile::max_ui_schema_depth as Configure),
        (20, CompilationProfile::max_ui_schema_nodes),
        (17, CompilationProfile::max_ui_schema_members),
        (3, CompilationProfile::max_ui_schema_collection_length),
        (
            namespace.len(),
            CompilationProfile::max_ui_schema_scalar_bytes,
        ),
    ] {
        assert!(
            FormDefinition::compiler(data_schema.clone())
                .ui_schema(ui_schema.clone())
                .profile(configure(CompilationProfile::default(), observed - 1))
                .compile()
                .is_err(),
            "pre-parsed UI schemas still enforce every post-parse limit"
        );
        FormDefinition::compiler(data_schema.clone())
            .ui_schema(ui_schema.clone())
            .profile(configure(CompilationProfile::default(), observed))
            .compile()
            .expect("the exact post-parse limit should compile");
        FormDefinition::compiler(data_schema.clone())
            .ui_schema(ui_schema.clone())
            .profile(configure(CompilationProfile::default(), observed + 1))
            .compile()
            .expect("the below post-parse limit should compile");
    }

    object_definition(json!({}))
        .form(json!({ "name": "Ada" }))
        .limits(FormDataLimits::default().max_bytes(0).max_tokens(0))
        .build()
        .expect("pre-parsed form data cannot be charged for source parsing");
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn extension_limits_have_below_exact_and_above_boundaries() {
    let namespace = "https://example.com/x";
    let bytes = format!(
        r#"{{
            "version": 1,
            "required_extensions": ["{namespace}"],
            "root": {{
                "type": "text",
                "value": {{
                    "extensions": {{"{namespace}": {{"nested": [true]}}}},
                    "content": {{"fallback": "Hello", "key": null}}
                }}
            }}
        }}"#
    );
    type Configure = fn(CompilationProfile, usize) -> CompilationProfile;
    let cases: [(&str, usize, Configure); 6] = [
        (
            "namespaces",
            1,
            CompilationProfile::max_extension_namespaces,
        ),
        (
            "occurrences",
            1,
            CompilationProfile::max_extension_occurrences,
        ),
        (
            "namespace_bytes",
            namespace.len(),
            CompilationProfile::max_extension_namespace_bytes,
        ),
        (
            "value_depth",
            3,
            CompilationProfile::max_extension_value_depth,
        ),
        (
            "value_nodes",
            3,
            CompilationProfile::max_extension_value_nodes,
        ),
        (
            "value_bytes",
            10,
            CompilationProfile::max_extension_value_bytes,
        ),
    ];

    for (dimension, observed, configure) in cases {
        for maximum in [observed - 1, observed, observed + 1] {
            let result = parse_ui_schema_v1(
                bytes.as_bytes(),
                &configure(CompilationProfile::default(), maximum),
            );
            assert_eq!(
                result.is_ok(),
                maximum >= observed,
                "extension {dimension} at {maximum}"
            );
            if maximum < observed {
                let path = match dimension {
                    "namespaces" | "namespace_bytes" => "/required_extensions/0",
                    _ => "/root/value/extensions/https:~1~1example.com~1x",
                };
                assert_limit(
                    parsed_resource_limit(result.unwrap_err()),
                    match dimension {
                        "namespaces" => "extension_namespaces",
                        "occurrences" => "extension_occurrences",
                        "namespace_bytes" => "extension_namespace_bytes",
                        "value_depth" => "extension_value_depth",
                        "value_nodes" => "extension_value_nodes",
                        "value_bytes" => "extension_value_bytes",
                        _ => unreachable!(),
                    },
                    maximum,
                    observed,
                    path,
                );
            }
        }
    }

    let ui_schema = parse_ui_schema_v1(bytes.as_bytes(), &CompilationProfile::default()).unwrap();
    let data_schema = json!({
        "$schema": DIALECT,
        "type": "object",
        "additionalProperties": false,
        "properties": {}
    });
    for (_, observed, configure) in cases {
        assert!(
            FormDefinition::compiler(data_schema.clone())
                .ui_schema(ui_schema.clone())
                .profile(configure(CompilationProfile::default(), observed - 1))
                .compile()
                .is_err(),
            "pre-parsed extensions enforce the above-limit boundary"
        );
        FormDefinition::compiler(data_schema.clone())
            .ui_schema(ui_schema.clone())
            .profile(configure(CompilationProfile::default(), observed))
            .compile()
            .expect("pre-parsed extensions accept the exact boundary");
        FormDefinition::compiler(data_schema.clone())
            .ui_schema(ui_schema.clone())
            .profile(configure(CompilationProfile::default(), observed + 1))
            .compile()
            .expect("pre-parsed extensions accept the below-limit boundary");
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn initial_form_tree_limits_fail_before_a_form_is_published() {
    let definition = object_definition(json!({
        "people": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "name": { "type": "string" },
                    "city": { "type": "string" }
                }
            }
        }
    }));
    let data = json!({ "people": [{ "name": "Ada", "city": "London" }] });

    for maximum in [0, 1, 2] {
        let result = definition
            .form(data.clone())
            .limits(FormDataLimits::default().max_repeated_items(maximum))
            .build();
        assert_eq!(result.is_ok(), maximum >= 1);
    }

    // Root + array control + three item-template nodes for one row.
    for maximum in [4, 5, 6] {
        let result = definition
            .form(data.clone())
            .limits(FormDataLimits::default().max_form_tree_nodes(maximum))
            .build();
        assert_eq!(result.is_ok(), maximum >= 5);
    }

    let failed = definition
        .form(data.clone())
        .limits(FormDataLimits::default().max_form_tree_nodes(4))
        .build();
    assert!(failed.is_err());
    let form = definition.create_form(data).unwrap();
    assert_eq!(form.form_data()["people"].as_array().unwrap().len(), 1);
}
