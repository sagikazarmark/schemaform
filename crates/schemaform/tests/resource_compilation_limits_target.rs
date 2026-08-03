use schemaform::{
    AnalysisError, CompilationLimitDimension, CompilationLimitPhase, CompilationProfile,
    CompileError, FormDefinition, JsonParseError, ResourceError, RetrievalUri, SchemaResource,
    json::parse_data_schema,
};
use serde_json::{Value, json};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";
type ConfigureLimit = fn(CompilationProfile, usize) -> CompilationProfile;

fn uri(value: &str) -> RetrievalUri {
    RetrievalUri::parse(value).expect("fixture URIs are valid")
}

fn object_schema(properties: Value) -> Value {
    json!({
        "$schema": DIALECT,
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
    })
}

fn compilation_limit(error: CompileError) -> schemaform::CompilationLimitError {
    let CompileError::Resource(ResourceError::Limit(error)) = error else {
        panic!("expected a compilation resource limit, got {error}");
    };
    error
}

fn compile_error(result: Result<FormDefinition, CompileError>) -> CompileError {
    match result {
        Ok(_) => panic!("compilation unexpectedly succeeded"),
        Err(error) => error,
    }
}

fn analysis_error(result: Result<schemaform::FormAnalysis, AnalysisError>) -> AnalysisError {
    match result {
        Ok(_) => panic!("analysis unexpectedly succeeded"),
        Err(error) => error,
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn parsed_bytes_and_tokens_have_below_exact_and_above_boundaries() {
    let bytes = br#"{"type":"object"}"#;
    for maximum in [bytes.len() - 1, bytes.len(), bytes.len() + 1] {
        let result = parse_data_schema(
            bytes,
            &CompilationProfile::default().max_data_schema_bytes(maximum),
        );
        if maximum < bytes.len() {
            let Err(JsonParseError::CompilationLimit(error)) = result else {
                panic!("the above-byte-limit input should fail")
            };
            assert_eq!(error.phase(), CompilationLimitPhase::Parse);
            assert_eq!(error.dimension(), CompilationLimitDimension::Bytes);
            assert_eq!(error.maximum(), maximum);
            assert_eq!(error.observed(), bytes.len());
            assert_eq!(error.pointer().as_str(), "");
        } else {
            assert!(result.is_ok());
        }
    }

    // Object, one member name, and one scalar value are three semantic JSON tokens.
    for maximum in [2, 3, 4] {
        let result = parse_data_schema(
            bytes,
            &CompilationProfile::default().max_data_schema_tokens(maximum),
        );
        if maximum < 3 {
            let Err(JsonParseError::CompilationLimit(error)) = result else {
                panic!("the above-token-limit input should fail")
            };
            assert_eq!(error.phase(), CompilationLimitPhase::Parse);
            assert_eq!(error.dimension(), CompilationLimitDimension::Tokens);
            assert_eq!(error.maximum(), maximum);
            assert_eq!(error.observed(), 3);
        } else {
            assert!(result.is_ok());
        }
    }

    let error = parse_data_schema(
        bytes,
        &CompilationProfile::default().max_data_schema_nodes(1),
    )
    .expect_err("parsed values also enforce post-parse structural limits");
    assert!(matches!(
        error,
        JsonParseError::CompilationLimit(ref limit)
            if limit.phase() == CompilationLimitPhase::Structure
                && limit.dimension() == CompilationLimitDimension::Nodes
                && limit.observed() == 2
    ));

    let deep = format!("{}null{}", "[".repeat(140), "]".repeat(140));
    let error = parse_data_schema(
        deep.as_bytes(),
        &CompilationProfile::default().max_data_schema_depth(64),
    )
    .expect_err("source depth is qualified before serde's recursion ceiling");
    assert!(matches!(
        error,
        JsonParseError::CompilationLimit(ref limit)
            if limit.phase() == CompilationLimitPhase::Structure
                && limit.dimension() == CompilationLimitDimension::Depth
                && limit.observed() == 140
    ));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn pre_parsed_structure_limits_have_below_exact_and_above_boundaries() {
    let schema = object_schema(json!({ "name": { "type": "string" } }));
    let cases: [(CompilationLimitDimension, usize, ConfigureLimit); 3] = [
        (
            CompilationLimitDimension::Depth,
            3,
            CompilationProfile::max_data_schema_depth,
        ),
        (
            CompilationLimitDimension::Nodes,
            7,
            CompilationProfile::max_data_schema_nodes,
        ),
        (
            CompilationLimitDimension::Members,
            4,
            CompilationProfile::max_data_schema_members,
        ),
    ];

    for (dimension, observed, configure) in cases {
        for maximum in [observed - 1, observed, observed + 1] {
            let result = FormDefinition::compiler(schema.clone())
                .profile(configure(CompilationProfile::default(), maximum))
                .compile();
            if maximum < observed {
                let error = compilation_limit(compile_error(result));
                assert_eq!(error.phase(), CompilationLimitPhase::Structure);
                assert_eq!(error.dimension(), dimension);
                assert_eq!(error.maximum(), maximum);
                assert_eq!(error.observed(), observed);
            } else {
                result.expect("the exact and below-limit schemas compile");
            }
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn pre_parsed_scalar_bytes_have_below_exact_and_above_boundaries() {
    let schema = object_schema(json!({ "name": { "type": "string", "title": "abc" } }));
    let observed = DIALECT.len();

    for maximum in [observed - 1, observed, observed + 1] {
        let result = FormDefinition::compiler(schema.clone())
            .profile(CompilationProfile::default().max_data_schema_scalar_bytes(maximum))
            .compile();
        if maximum < observed {
            let error = compilation_limit(compile_error(result));
            assert_eq!(error.phase(), CompilationLimitPhase::Structure);
            assert_eq!(error.dimension(), CompilationLimitDimension::ScalarBytes);
            assert_eq!(error.maximum(), maximum);
            assert_eq!(error.observed(), observed);
        } else {
            result.expect("the exact and below-limit scalar-byte schemas compile");
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn graph_limits_have_below_exact_and_above_boundaries() {
    let root = object_schema(json!({
        "name": { "$ref": "https://schemas.example/name.json" }
    }));
    let resource = SchemaResource::new(
        uri("https://schemas.example/name.json"),
        json!({ "$schema": DIALECT, "type": "string" }),
    );
    let cases: [(CompilationLimitDimension, usize, ConfigureLimit); 3] = [
        (
            CompilationLimitDimension::Resources,
            2,
            CompilationProfile::max_data_schema_resources,
        ),
        (
            CompilationLimitDimension::References,
            1,
            CompilationProfile::max_data_schema_references,
        ),
        (
            CompilationLimitDimension::Traversal,
            5,
            CompilationProfile::max_data_schema_traversal,
        ),
    ];

    for (dimension, observed, configure) in cases {
        for maximum in [observed - 1, observed, observed + 1] {
            let result = FormDefinition::compiler(root.clone())
                .root_uri(uri("https://schemas.example/root.json"))
                .resource(resource.clone())
                .profile(configure(CompilationProfile::default(), maximum))
                .compile();
            if maximum < observed {
                let error = compilation_limit(compile_error(result));
                assert_eq!(error.phase(), CompilationLimitPhase::Graph);
                assert_eq!(error.dimension(), dimension);
                assert_eq!(error.maximum(), maximum);
                assert_eq!(error.observed(), observed);
            } else {
                result.expect("the exact and below-limit graphs compile");
            }
        }
    }

    let opaque_target = json!({
        "$schema": DIALECT,
        "$ref": "#/x-projected-root",
        "x-projected-root": {
            "$id": "projected.json",
            "type": "object",
            "additionalProperties": false,
            "properties": { "name": { "type": "string" } }
        }
    });
    let error = compilation_limit(compile_error(
        FormDefinition::compiler(opaque_target)
            .root_uri(uri("https://schemas.example/root.json"))
            .profile(CompilationProfile::default().max_data_schema_resources(1))
            .compile(),
    ));
    assert_eq!(error.phase(), CompilationLimitPhase::Graph);
    assert_eq!(error.dimension(), CompilationLimitDimension::Resources);
    assert_eq!(error.observed(), 2);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn output_limits_have_below_exact_and_above_boundaries() {
    let schema = object_schema(json!({
        "first": { "type": "string" },
        "second": { "type": "integer" }
    }));
    let cases: [(CompilationLimitDimension, usize, ConfigureLimit); 2] = [
        (
            CompilationLimitDimension::DefinitionNodes,
            3,
            CompilationProfile::max_definition_nodes,
        ),
        (
            CompilationLimitDimension::Controls,
            2,
            CompilationProfile::max_controls,
        ),
    ];

    for (dimension, observed, configure) in cases {
        for maximum in [observed - 1, observed, observed + 1] {
            let result = FormDefinition::compiler(schema.clone())
                .profile(configure(CompilationProfile::default(), maximum))
                .compile();
            if maximum < observed {
                let error = compilation_limit(compile_error(result));
                assert_eq!(error.phase(), CompilationLimitPhase::Definition);
                assert_eq!(error.dimension(), dimension);
                assert_eq!(error.maximum(), maximum);
                assert_eq!(error.observed(), observed);
            } else {
                result.expect("the exact and below-limit outputs compile");
            }
        }
    }

    let first = FormDefinition::compiler(schema.clone())
        .profile(CompilationProfile::default().max_controls(2))
        .compile()
        .expect("the exact control limit compiles");
    let second = FormDefinition::compiler(schema)
        .profile(CompilationProfile::default().max_controls(3))
        .compile()
        .expect("the below control limit compiles");
    assert_ne!(first.fingerprint(), second.fingerprint());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn uri_pointer_and_capability_limits_have_boundaries() {
    let unsupported = object_schema(json!({
        "long": { "oneOf": [{ "type": "string" }, { "type": "integer" }] }
    }));
    let uri_bytes = DIALECT.len();
    let pointer_bytes = "/properties/long/oneOf/0/type".len();
    let cases: [(CompilationLimitDimension, usize, ConfigureLimit); 2] = [
        (
            CompilationLimitDimension::UriBytes,
            uri_bytes,
            CompilationProfile::max_uri_bytes,
        ),
        (
            CompilationLimitDimension::PointerBytes,
            pointer_bytes,
            CompilationProfile::max_pointer_bytes,
        ),
    ];
    for (dimension, observed, configure) in cases {
        for maximum in [observed - 1, observed, observed + 1] {
            let result = FormDefinition::compiler(unsupported.clone())
                .profile(configure(CompilationProfile::default(), maximum))
                .analyze();
            if maximum < observed {
                let AnalysisError::Resource(ResourceError::Limit(error)) = analysis_error(result)
                else {
                    panic!("analysis returned the wrong error")
                };
                assert_eq!(error.dimension(), dimension);
                assert_eq!(error.observed(), observed);
            } else {
                result.expect("the exact and below-limit analyses succeed");
            }
        }
    }

    for maximum in [0, 1, 2] {
        let result = FormDefinition::compiler(unsupported.clone())
            .profile(CompilationProfile::default().max_capability_findings(maximum))
            .analyze();
        if maximum == 0 {
            let AnalysisError::Resource(ResourceError::Limit(error)) = analysis_error(result)
            else {
                panic!("analysis returned the wrong error")
            };
            assert_eq!(error.phase(), CompilationLimitPhase::Definition);
            assert_eq!(
                error.dimension(),
                CompilationLimitDimension::CapabilityFindings
            );
            assert_eq!(error.observed(), 1);
        } else {
            let analysis = result.expect("the exact and below-limit reports are complete");
            assert_eq!(analysis.capability_report().findings().count(), 1);
        }
    }

    let root_uri = "https://schemas.example/a/very/long/qualification/path/root.json";
    let resolved = "https://schemas.example/a/very/long/qualification/path/child.json";
    let nested_resource = object_schema(json!({
        "nested": {
            "$id": "child.json",
            "type": "object",
            "additionalProperties": false,
            "properties": {}
        }
    }));
    for maximum in [resolved.len() - 1, resolved.len(), resolved.len() + 1] {
        let result = FormDefinition::compiler(nested_resource.clone())
            .root_uri(uri(root_uri))
            .profile(CompilationProfile::default().max_uri_bytes(maximum))
            .compile();
        if maximum < resolved.len() {
            let error = compilation_limit(compile_error(result));
            assert_eq!(error.dimension(), CompilationLimitDimension::UriBytes);
            assert_eq!(error.observed(), resolved.len());
        } else {
            result.expect("resolved canonical identities at the limit compile");
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn phase_and_dimension_precedence_is_deterministic() {
    let bytes = br#"{"type":"object"}"#;
    let error = parse_data_schema(
        bytes,
        &CompilationProfile::default()
            .max_data_schema_bytes(bytes.len() - 1)
            .max_data_schema_tokens(0),
    )
    .expect_err("both parse limits fail");
    assert!(matches!(
        error,
        JsonParseError::CompilationLimit(ref limit)
            if limit.dimension() == CompilationLimitDimension::Bytes
    ));

    let error = compilation_limit(compile_error(
        FormDefinition::compiler(object_schema(json!({ "name": { "type": "string" } })))
            .profile(
                CompilationProfile::default()
                    .max_data_schema_depth(0)
                    .max_data_schema_nodes(0)
                    .max_data_schema_resources(0)
                    .max_definition_nodes(0),
            )
            .compile(),
    ));
    assert_eq!(error.phase(), CompilationLimitPhase::Structure);
    assert_eq!(error.dimension(), CompilationLimitDimension::Depth);

    let profile = CompilationProfile::default()
        .max_data_schema_resources(0)
        .max_definition_nodes(0);
    let strict_limit = compilation_limit(compile_error(
        FormDefinition::compiler(object_schema(json!({})))
            .profile(profile.clone())
            .compile(),
    ));
    let AnalysisError::Resource(ResourceError::Limit(analysis_error)) = analysis_error(
        FormDefinition::compiler(object_schema(json!({})))
            .profile(profile)
            .analyze(),
    ) else {
        panic!("analysis returned the wrong error")
    };
    assert_eq!(strict_limit, analysis_error);
    assert_eq!(strict_limit.phase(), CompilationLimitPhase::Graph);
    assert_eq!(
        strict_limit.dimension(),
        CompilationLimitDimension::Resources
    );

    let projection_error = compilation_limit(compile_error(
        FormDefinition::compiler(object_schema(json!({
            "first": { "type": "string" },
            "second": { "type": "string" }
        })))
        .profile(CompilationProfile::default().max_data_schema_traversal(4))
        .compile(),
    ));
    assert_eq!(projection_error.phase(), CompilationLimitPhase::Projection);
    assert_eq!(
        projection_error.dimension(),
        CompilationLimitDimension::Traversal
    );
    assert_eq!(projection_error.maximum(), 4);
    assert_eq!(projection_error.observed(), 5);
}
