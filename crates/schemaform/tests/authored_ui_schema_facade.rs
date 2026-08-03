use schemaform::{
    CompilationProfile, Dialect, ExtensionNamespace, FormDefinition, JsonPointer,
    ResourceLimitError, ResourceLimitPhase, SubmissionOutcome,
    definition::{
        CompileError, DefinitionNodeKind, InputError, SemanticKind, UiSchemaInputErrorKind,
    },
    json::{JsonParseError, parse_ui_schema_v1},
};
use serde_json::json;
use sha2::{Digest, Sha256};

const COMPLETE_UI_SCHEMA_V1: &[u8] = include_bytes!("fixtures/ui-schema-v1/complete.json");
const REJECTED_UI_SCHEMA_V1: &[u8] = include_bytes!("fixtures/ui-schema-v1/rejected.json");
const UI_SCHEMA_V1_META_SCHEMA: &[u8] = include_bytes!("../../../ui-schema-v1.schema.json");
const QUALIFIED_UI_SCHEMA_V0_SHAPE_SHA256: &str =
    "30a801e563a414b1a10c0d184d514554794c45c8cfcb88bd6968aa7381eb7eb2";

fn resource_limit(error: JsonParseError) -> ResourceLimitError {
    let JsonParseError::ResourceLimit(error) = error else {
        panic!("expected a JSON resource limit, got {error:?}");
    };
    assert_eq!(error.phase(), ResourceLimitPhase::Construction);
    error
}

fn invalid_ui_schema(error: JsonParseError) -> (JsonPointer, String) {
    let JsonParseError::InvalidUiSchema { location, reason } = error else {
        panic!("expected an invalid UI-schema diagnostic")
    };
    assert!(
        !reason.is_empty(),
        "invalid UI-schema diagnostics retain a reason"
    );
    (location, reason)
}

#[test]
fn ui_schema_v1_is_the_stable_wire_version() {
    for version in ["1", "1.0", "1e0", "10e-1"] {
        let document = format!(
            r#"{{
                "version": {version},
                "root": {{
                    "type": "text",
                    "value": {{ "content": {{ "fallback": "Hello" }} }}
                }}
            }}"#
        );
        let ui_schema = parse_ui_schema_v1(document.as_bytes(), &CompilationProfile::default())
            .unwrap_or_else(|error| {
                panic!("semantic integer version {version} should parse: {error}")
            });

        assert_eq!(serde_json::to_value(ui_schema).unwrap()["version"], 1);
    }
}

#[test]
fn complete_stable_ui_schema_v1_fixture_round_trips_and_compiles() {
    let profile = CompilationProfile::default();
    let mut stable_wire: serde_json::Value = serde_json::from_slice(COMPLETE_UI_SCHEMA_V1).unwrap();
    stable_wire
        .as_object_mut()
        .expect("the accepted UI schema should be an object")
        .remove("version");
    assert_eq!(
        hex_digest(&serde_json::to_vec(&stable_wire).unwrap()),
        QUALIFIED_UI_SCHEMA_V0_SHAPE_SHA256,
        "the stable fixture must preserve the qualified v0 shape apart from its discriminator"
    );

    let ui_schema = parse_ui_schema_v1(COMPLETE_UI_SCHEMA_V1, &profile)
        .expect("the complete stable v1 fixture should parse");
    let encoded = serde_json::to_vec(&ui_schema).expect("the v1 fixture should serialize");
    assert_eq!(
        parse_ui_schema_v1(&encoded, &profile).expect("the serialized fixture should parse"),
        ui_schema,
        "every accepted v1 wire value must round-trip semantically"
    );

    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "count": { "type": "integer" },
            "active": { "type": "boolean" },
            "choice": { "enum": ["one", "two"] },
            "tags": {
                "type": "array",
                "items": { "type": "string" }
            },
            "extra": { "type": "string" }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the complete stable v1 fixture should compile");

    let mut pending = vec![definition.root()];
    let mut authored_ids = Vec::new();
    let mut item_label = None;
    while let Some(id) = pending.pop() {
        let node = definition.node(id).unwrap();
        authored_ids.extend(node.authored_id().map(str::to_owned));
        if node
            .binding()
            .is_some_and(|binding| binding.as_str() == "/tags")
        {
            item_label = node.item_label_reference().cloned();
        }
        pending.extend(node.children());
    }
    authored_ids.sort();
    assert_eq!(
        authored_ids,
        [
            "active-control",
            "choice-control",
            "count-control",
            "details-tabs",
            "facts-grid",
            "identity-group",
            "intro",
            "name-control",
            "remaining-auto",
            "root-stack",
            "tag-control",
            "tag-help",
            "tag-template",
            "tags-control",
        ]
    );
    let item_label = item_label.expect("the array item label should be retained");
    assert_eq!(item_label.fallback(), "Tag");
    assert_eq!(item_label.key(), Some("form.tag"));
}

#[test]
fn empty_widget_symbols_are_rejected_during_ui_schema_parsing() {
    let error = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "/name" },
                    "widget": ""
                }
            }
        }"#,
        &CompilationProfile::default(),
    )
    .expect_err("an empty widget symbol must fail at the wire boundary");
    let JsonParseError::InvalidUiSchema { location, reason } = error else {
        panic!("an empty widget symbol must report an invalid UI schema")
    };
    assert_eq!(location, JsonPointer::parse("/root/value/widget").unwrap());
    assert!(reason.contains("invalid public address: Empty"));
}

#[test]
fn stable_ui_schema_v1_meta_schema_and_strict_failure_fixtures_agree() {
    let meta_schema: serde_json::Value = serde_json::from_slice(UI_SCHEMA_V1_META_SCHEMA).unwrap();
    jsonschema::draft202012::meta::validate(&meta_schema)
        .expect("the stable v1 meta-schema should be a valid Draft 2020-12 schema");
    let validator = jsonschema::draft202012::new(&meta_schema)
        .expect("the stable v1 meta-schema should compile");
    let complete: serde_json::Value = serde_json::from_slice(COMPLETE_UI_SCHEMA_V1).unwrap();
    assert!(validator.is_valid(&complete));

    let rejected: serde_json::Value = serde_json::from_slice(REJECTED_UI_SCHEMA_V1).unwrap();
    assert_eq!(rejected["wire_version"], 1);
    assert_eq!(rejected["stable_headless_compatibility"], true);
    assert_eq!(
        rejected["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| case["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "obsolete version 0",
            "future version 2",
            "non-integer version 1.5",
            "unknown element",
            "unknown grid-cell field",
            "binding pointer is not RFC 6901",
            "empty widget symbol",
            "relative extension namespace",
            "malformed absolute extension namespace",
            "text reference without a fallback",
            "text reference with an invalid key type",
            "text reference with an unknown field",
            "required extension is absent",
            "template binding outside an item template",
            "item template on a scalar control",
            "duplicate element ID",
            "duplicate editor binding",
            "Auto selects an unknown property",
        ],
        "the stable rejection corpus must retain every qualified v0 boundary"
    );
    let profile = CompilationProfile::default();
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "rows": { "type": "array", "items": { "type": "string" } }
        }
    });
    for case in rejected["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let document = &case["document"];
        if case["meta_schema_rejects"] == true {
            assert!(!validator.is_valid(document), "meta-schema accepted {name}");
        }
        let bytes = serde_json::to_vec(document).unwrap();
        let location = case["location"].as_str().unwrap();
        match case["phase"].as_str().unwrap() {
            "parse" => {
                let error = parse_ui_schema_v1(&bytes, &profile)
                    .expect_err("the strict wire fixture must fail parsing");
                let (actual, _) = invalid_ui_schema(error);
                assert_eq!(actual.as_str(), location, "unexpected location for {name}");
            }
            "compile" => {
                let ui_schema = parse_ui_schema_v1(&bytes, &profile).unwrap_or_else(|error| {
                    panic!("compile fixture {name} did not parse: {error:?}")
                });
                let Err(CompileError::Input(InputError::InvalidUiSchema(error))) =
                    FormDefinition::compiler(data_schema.clone())
                        .ui_schema(ui_schema)
                        .compile()
                else {
                    panic!("compile fixture {name} did not produce a UI-schema input error")
                };
                assert_eq!(error.location().as_str(), location, "location for {name}");
                assert_eq!(
                    format!("{:?}", error.kind()),
                    case["kind"].as_str().unwrap(),
                    "kind for {name}"
                );
            }
            phase => panic!("unknown fixture phase {phase}"),
        }
    }
}

#[test]
fn authored_element_ids_are_preserved_fingerprinted_and_unique() {
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "name": { "type": "string" } }
    });
    let compile = |root_id: &str, text_id: &str| {
        let ui_schema = parse_ui_schema_v1(
            format!(
                r#"{{
                    "version": 1,
                    "root": {{
                        "type": "stack",
                        "value": {{
                            "id": "{root_id}",
                            "children": [
                                {{
                                    "type": "text",
                                    "value": {{
                                        "id": "{text_id}",
                                        "content": {{ "fallback": "Intro" }}
                                    }}
                                }},
                                {{
                                    "type": "control",
                                    "value": {{
                                        "id": "name-control",
                                        "binding": {{ "origin": "root", "pointer": "/name" }}
                                    }}
                                }}
                            ]
                        }}
                    }}
                }}"#
            )
            .as_bytes(),
            &CompilationProfile::default(),
        )
        .unwrap();
        FormDefinition::compiler(data_schema.clone())
            .ui_schema(ui_schema)
            .compile()
    };

    let definition = compile("root-stack", "intro").expect("unique element IDs should compile");
    let stack = definition
        .node(definition.root())
        .unwrap()
        .children()
        .next()
        .and_then(|id| definition.node(id))
        .unwrap();
    assert_eq!(stack.authored_id(), Some("root-stack"));
    assert_eq!(
        stack
            .children()
            .map(|id| definition.node(id).unwrap().authored_id())
            .collect::<Vec<_>>(),
        [Some("intro"), Some("name-control")]
    );
    assert_ne!(
        definition.fingerprint(),
        compile("other-root", "intro").unwrap().fingerprint(),
        "authored IDs are semantic definition input"
    );

    let Err(CompileError::Input(InputError::InvalidUiSchema(error))) =
        compile("duplicate", "duplicate")
    else {
        panic!("duplicate element IDs should be a UI-schema input error")
    };
    assert_eq!(error.location().as_str(), "/root/value/children/0/value/id");
    assert_eq!(error.kind(), UiSchemaInputErrorKind::DuplicateElementId);
}

#[test]
fn exact_uri_extensions_are_validated_preserved_and_fingerprinted() {
    let profile = CompilationProfile::default();
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "name": { "type": "string" } }
    });
    let compile = |value: serde_json::Value| {
        let ui_schema = parse_ui_schema_v1(
            serde_json::to_vec(&json!({
                "version": 1,
                "required_extensions": ["https://example.com/z", "https://example.com/a"],
                "root": {
                    "type": "control",
                    "value": {
                        "binding": { "origin": "root", "pointer": "/name" },
                        "extensions": {
                            "https://example.com/z": value,
                            "https://example.com/a": { "unknown": [true, null, 7] }
                        }
                    }
                }
            }))
            .unwrap()
            .as_slice(),
            &profile,
        )
        .expect("valid exact-URI extensions should parse");
        FormDefinition::compiler(data_schema.clone())
            .ui_schema(ui_schema)
            .compile()
            .expect("valid extensions should compile")
    };

    let definition = compile(json!({ "enabled": true }));
    assert_eq!(
        definition
            .required_extensions()
            .map(ExtensionNamespace::as_str)
            .collect::<Vec<_>>(),
        ["https://example.com/a", "https://example.com/z"]
    );
    let control = definition
        .node(definition.root())
        .unwrap()
        .children()
        .next()
        .and_then(|id| definition.node(id))
        .unwrap();
    assert_eq!(
        control
            .extensions()
            .map(|(namespace, value)| (namespace.as_str(), value.clone()))
            .collect::<Vec<_>>(),
        [
            (
                "https://example.com/a",
                json!({ "unknown": [true, null, 7] })
            ),
            ("https://example.com/z", json!({ "enabled": true }))
        ]
    );
    assert_eq!(
        definition.fingerprint(),
        compile(json!({ "enabled": true })).fingerprint()
    );
    assert_ne!(
        definition.fingerprint(),
        compile(json!({ "enabled": false })).fingerprint()
    );

    for invalid in ["relative/path", "not a uri:with spaces"] {
        assert!(ExtensionNamespace::parse(invalid).is_err());
        let document = format!(
            r#"{{
                "version": 1,
                "root": {{
                    "type": "text",
                    "value": {{
                        "content": {{ "fallback": "Hi" }},
                        "extensions": {{ "{invalid}": true }}
                    }}
                }}
            }}"#
        );
        let result = parse_ui_schema_v1(document.as_bytes(), &profile);
        assert!(
            matches!(result, Err(JsonParseError::InvalidUiSchema { .. })),
            "unexpected result for {invalid:?}: {result:?}"
        );
    }
}

#[test]
fn required_extensions_must_be_unique_present_and_bounded() {
    let parse = |required: &str, extensions: &str, profile: &CompilationProfile| {
        parse_ui_schema_v1(
            format!(
                r#"{{
                    "version": 1,
                    "required_extensions": [{required}],
                    "root": {{
                        "type": "text",
                        "value": {{
                            "content": {{ "fallback": "Hi" }},
                            "extensions": {{ {extensions} }}
                        }}
                    }}
                }}"#
            )
            .as_bytes(),
            profile,
        )
    };

    let (location, reason) = invalid_ui_schema(
        parse(
            r#""https://example.com/a", "https://example.com/a""#,
            r#""https://example.com/a": true"#,
            &CompilationProfile::default(),
        )
        .unwrap_err(),
    );
    assert_eq!(location.as_str(), "/required_extensions/1");
    assert_eq!(reason, "duplicate required extension");
    let (location, _) = invalid_ui_schema(
        parse(
            r#""https://example.com/missing""#,
            r#""https://example.com/optional": true"#,
            &CompilationProfile::default(),
        )
        .unwrap_err(),
    );
    assert_eq!(location.as_str(), "/required_extensions/0");

    let bounded = CompilationProfile::default().max_extension_namespaces(1);
    let limit = resource_limit(
        parse(
            r#""https://example.com/a", "https://example.com/b""#,
            r#""https://example.com/a": true, "https://example.com/b": true"#,
            &bounded,
        )
        .unwrap_err(),
    );
    assert_eq!(limit.dimension(), "extension_namespaces");
    assert_eq!(limit.maximum(), 1);
    assert_eq!(limit.observed(), 2);
    assert_eq!(limit.path().as_str(), "/required_extensions/1");

    assert!(matches!(
        parse_ui_schema_v1(
            br#"{
                "version": 1,
                "root": {
                    "type": "text",
                    "value": {
                        "content": { "fallback": "Hi" },
                        "extensions": {
                            "https://example.com/duplicate": true,
                            "https://example.com/duplicate": false
                        }
                    }
                }
            }"#,
            &CompilationProfile::default(),
        ),
        Err(JsonParseError::InvalidUiSchema { .. })
    ));

    let bounded = CompilationProfile::default().max_extension_value_nodes(2);
    let limit = resource_limit(
        parse(
            "",
            r#""https://example.com/optional": { "one": true, "two": false }"#,
            &bounded,
        )
        .unwrap_err(),
    );
    assert_eq!(limit.dimension(), "extension_value_nodes");
    assert_eq!(limit.maximum(), 2);
    assert_eq!(limit.observed(), 3);
    assert_eq!(
        limit.path().as_str(),
        "/root/value/extensions/https:~1~1example.com~1optional"
    );
    assert!(
        parse(
            "",
            r#""https://example.com/a": { "one": true }, "https://example.com/b": { "two": false }"#,
            &bounded,
        )
        .is_ok(),
        "extension value node limits apply independently to each occurrence"
    );

    let bounded = CompilationProfile::default().max_extension_value_bytes(3);
    let limit = resource_limit(
        parse("", r#""https://example.com/optional": "four""#, &bounded).unwrap_err(),
    );
    assert_eq!(limit.dimension(), "extension_value_bytes");
    assert_eq!(limit.maximum(), 3);
    assert_eq!(limit.observed(), 4);
    assert_eq!(
        limit.path().as_str(),
        "/root/value/extensions/https:~1~1example.com~1optional"
    );
    assert!(
        parse(
            "",
            r#""https://example.com/a": "one", "https://example.com/b": "two""#,
            &bounded,
        )
        .is_ok(),
        "extension value byte limits apply independently to each occurrence"
    );

    let bounded = CompilationProfile::default().max_ui_schema_bytes(10);
    let limit = resource_limit(parse("", "", &bounded).unwrap_err());
    assert_eq!(limit.dimension(), "bytes");
    assert_eq!(limit.maximum(), 10);
    assert_eq!(limit.path().as_str(), "");
    assert!(
        limit.observed() > limit.maximum(),
        "source bytes must be rejected before deserialization can allocate an extension value"
    );
}

#[test]
fn ui_schema_v1_is_strict_and_reports_structured_locations() {
    let profile = CompilationProfile::default();
    let schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "/name" },
                    "label": {
                        "value": { "fallback": "Display name", "key": "profile.name" }
                    }
                }
            }
        }"#,
        &profile,
    )
    .expect("the strict UI schema should parse");
    assert_eq!(
        serde_json::to_value(&schema).unwrap()["version"],
        serde_json::Value::Number(1.into())
    );
    assert_eq!(
        schema
            .root()
            .control()
            .unwrap()
            .binding()
            .pointer()
            .as_str(),
        "/name"
    );
    let control: schemaform::ui::v1::Control = serde_json::from_value(json!({
        "binding": { "origin": "root", "pointer": "/name" }
    }))
    .expect("individual public UI-schema types should remain deserializable");
    assert_eq!(control.binding().pointer().as_str(), "/name");
    assert!(
        serde_json::from_value::<schemaform::ui::v1::Text>(json!({
            "content": { "fallback": "Hi" },
            "unexpected": true
        }))
        .is_err(),
        "individual public UI-schema types must remain strict"
    );

    let error = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "/name" },
                    "unexpected": true
                }
            }
        }"#,
        &profile,
    )
    .expect_err("unknown core fields must fail");
    let (location, _) = invalid_ui_schema(error);
    assert_eq!(location.as_str(), "/root/value/unexpected");

    let error = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "/rows" },
                    "item_template": {
                        "type": "control",
                        "value": {
                            "binding": { "origin": "item_template", "pointer": "" },
                            "unexpected": true
                        }
                    }
                }
            }
        }"#,
        &profile,
    )
    .expect_err("unknown item-template fields must fail at their nested location");
    let (location, _) = invalid_ui_schema(error);
    assert_eq!(
        location.as_str(),
        "/root/value/item_template/value/unexpected"
    );

    let error = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "name" }
                }
            }
        }"#,
        &profile,
    )
    .expect_err("bindings must be RFC 6901 JSON Pointers");
    let (location, _) = invalid_ui_schema(error);
    assert_eq!(location.as_str(), "/root/value/binding/pointer");

    for invalid_version in [r#""1""#, "0", "2", "1.0000000000000000000000001"] {
        let document = format!(
            r#"{{
                "version": {invalid_version},
                "root": {{
                    "type": "text",
                    "value": {{ "content": {{ "fallback": "Hello" }} }}
                }}
            }}"#
        );
        assert!(matches!(
            parse_ui_schema_v1(document.as_bytes(), &profile),
            Err(JsonParseError::InvalidUiSchema { .. })
        ));
    }

    let error = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": { "type": "card", "value": {} }
        }"#,
        &profile,
    )
    .expect_err("unknown element types must fail");
    let (location, _) = invalid_ui_schema(error);
    assert_eq!(location.as_str(), "/root/type");

    let error = parse_ui_schema_v1(
        br#"{
                "version": 1,
                "root": { "type": "text", "value": { "content": { "fallback": "Hi" } } }
            } {}"#,
        &profile,
    )
    .expect_err("a UI schema must contain exactly one JSON document");
    let JsonParseError::Syntax(syntax) = error else {
        panic!("trailing JSON must be a syntax diagnostic");
    };
    assert_eq!(syntax.reason(), "trailing characters");
    assert!(syntax.line() > 0);
    assert!(syntax.column() > 0);

    let error = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "elsewhere", "pointer": "/name" }
                }
            }
        }"#,
        &profile,
    )
    .expect_err("unknown binding origins must fail at their own location");
    let (location, _) = invalid_ui_schema(error);
    assert_eq!(location.as_str(), "/root/value/binding/origin");

    let error = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "text",
                "value": {
                    "content": { "fallback": "Hi" },
                    "value": true
                }
            }
        }"#,
        &profile,
    )
    .expect_err("repeated path tokens must remain distinct");
    let (location, _) = invalid_ui_schema(error);
    assert_eq!(location.as_str(), "/root/value/value");
}

#[test]
fn authored_widget_symbols_are_preserved_exactly_in_the_definition() {
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "tags": {
                "type": "array",
                "items": { "type": "string" }
            }
        }
    });
    let compile = |name_widget: &str| {
        let ui_schema = parse_ui_schema_v1(
            format!(
                r#"{{
                    "version": 1,
                    "root": {{
                        "type": "stack",
                        "value": {{
                            "children": [
                                {{
                                    "type": "control",
                                    "value": {{
                                        "binding": {{ "origin": "root", "pointer": "/name" }},
                                        "widget": "{name_widget}"
                                    }}
                                }},
                                {{
                                    "type": "control",
                                    "value": {{
                                        "binding": {{ "origin": "root", "pointer": "/tags" }},
                                        "widget": "company:tags",
                                        "item_template": {{
                                            "type": "control",
                                            "value": {{
                                                "binding": {{
                                                    "origin": "item_template",
                                                    "pointer": ""
                                                }},
                                                "widget": "company:tag"
                                            }}
                                        }}
                                    }}
                                }}
                            ]
                        }}
                    }}
                }}"#
            )
            .as_bytes(),
            &CompilationProfile::default(),
        )
        .expect("exact widget symbols should parse");
        FormDefinition::compiler(data_schema.clone())
            .ui_schema(ui_schema)
            .compile()
            .expect("authored widget symbols should compile")
    };

    let definition = compile("company:text");
    let stack = definition
        .node(definition.root())
        .unwrap()
        .children()
        .next()
        .unwrap();
    let controls = definition
        .node(stack)
        .unwrap()
        .children()
        .map(|id| definition.node(id).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        controls[0].widget().map(ToString::to_string).as_deref(),
        Some("company:text")
    );
    assert_eq!(
        controls[1].widget().map(ToString::to_string).as_deref(),
        Some("company:tags")
    );
    let item_template = controls[1]
        .children()
        .next()
        .and_then(|id| definition.node(id))
        .expect("the array should retain its item template");
    assert_eq!(
        item_template.widget().map(ToString::to_string).as_deref(),
        Some("company:tag")
    );
    assert_eq!(
        definition.fingerprint(),
        compile("company:text").fingerprint()
    );
    assert_ne!(
        definition.fingerprint(),
        compile("company:textarea").fingerprint()
    );

    let generated = FormDefinition::compile(data_schema).unwrap();
    assert!(
        generated
            .node(generated.root())
            .unwrap()
            .children()
            .filter_map(|id| generated.node(id))
            .all(|node| node.widget().is_none())
    );
}

#[test]
fn authored_ui_schema_preserves_order_omissions_and_form_semantics() {
    let profile = CompilationProfile::default();
    let ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "stack",
                "value": {
                    "children": [
                        {
                            "type": "text",
                            "value": {
                                "content": {
                                    "fallback": "Use <strong>plain text</strong>.",
                                    "key": "profile.intro"
                                }
                            }
                        },
                        {
                            "type": "control",
                            "value": {
                                "binding": { "origin": "root", "pointer": "/second" },
                                "label": {
                                    "value": {
                                        "fallback": "Second field",
                                        "key": "profile.second"
                                    }
                                }
                            }
                        },
                        {
                            "type": "group",
                            "value": {
                                "title": {
                                    "fallback": "Primary details",
                                    "key": "profile.primary"
                                },
                                "child": {
                                    "type": "control",
                                    "value": {
                                        "binding": { "origin": "root", "pointer": "/first" },
                                        "help": "suppress"
                                    }
                                }
                            }
                        }
                    ]
                }
            }
        }"#,
        &profile,
    )
    .unwrap();
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["first", "second", "hidden"],
        "properties": {
            "first": { "type": "string", "title": "First", "description": "Shown help" },
            "second": { "type": "string", "title": "Second" },
            "hidden": { "type": "string", "minLength": 1 }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored UI schema should compile");

    let root = definition.node(definition.root()).unwrap();
    let stack_id = root.children().next().expect("one authored semantic root");
    assert_eq!(root.children().count(), 1);
    let stack = definition.node(stack_id).unwrap();
    assert_eq!(stack.kind(), DefinitionNodeKind::Stack);
    let children = stack.children().collect::<Vec<_>>();
    assert_eq!(children.len(), 3);

    let text = definition.node(children[0]).unwrap();
    assert_eq!(text.kind(), DefinitionNodeKind::Text);
    assert_eq!(
        text.text().unwrap().fallback(),
        "Use <strong>plain text</strong>."
    );
    assert_eq!(text.text().unwrap().key(), Some("profile.intro"));

    let second = definition.node(children[1]).unwrap();
    assert_eq!(second.kind(), DefinitionNodeKind::Control);
    assert_eq!(second.binding().unwrap().as_str(), "/second");
    assert_eq!(second.label(), "Second field");
    assert_eq!(
        second.label_reference().unwrap().key(),
        Some("profile.second")
    );

    let group = definition.node(children[2]).unwrap();
    assert_eq!(group.kind(), DefinitionNodeKind::Group);
    assert_eq!(group.label(), "Primary details");
    assert_eq!(
        group.label_reference().unwrap().key(),
        Some("profile.primary")
    );
    let first = definition.node(group.children().next().unwrap()).unwrap();
    assert_eq!(first.binding().unwrap().as_str(), "/first");
    assert_eq!(first.label(), "First");
    assert_eq!(first.help(), None);

    let all_bindings = [stack_id]
        .into_iter()
        .chain(children.iter().copied())
        .chain(group.children())
        .filter_map(|id| {
            definition
                .node(id)
                .unwrap()
                .binding()
                .map(|binding| binding.as_str())
        })
        .collect::<Vec<_>>();
    assert!(!all_bindings.contains(&"/hidden"));

    let mut form = definition
        .create_form(json!({ "first": "Ada", "second": "Lovelace", "hidden": "" }))
        .unwrap();
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));
    let hidden_finding = form
        .view()
        .visible_findings()
        .find_map(|finding| match finding {
            schemaform::FindingView::Validation { target, finding }
                if finding.code() == "minLength" =>
            {
                Some(target)
            }
            _ => None,
        })
        .expect("the omitted location must still be validated");
    assert_eq!(hidden_finding, form.view().root());
    assert_eq!(form.form_data()["hidden"], "");
}

#[test]
fn authored_grid_validates_and_compiles_responsive_semantic_cells() {
    let profile = CompilationProfile::default();
    let ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "grid",
                "value": {
                    "cells": [
                        {
                            "compact_span": 12,
                            "wide_span": 4,
                            "child": {
                                "type": "control",
                                "value": {
                                    "binding": { "origin": "root", "pointer": "/first" }
                                }
                            }
                        },
                        {
                            "compact_span": 6,
                            "child": {
                                "type": "control",
                                "value": {
                                    "binding": { "origin": "root", "pointer": "/second" }
                                }
                            }
                        }
                    ]
                }
            }
        }"#,
        &profile,
    )
    .expect("valid grid spans should parse");
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "first": { "type": "string" },
            "second": { "type": "string" }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored grid should compile");

    let grid = definition
        .node(definition.root())
        .unwrap()
        .children()
        .next()
        .and_then(|id| definition.node(id))
        .expect("the semantic grid should be present");
    assert_eq!(grid.kind(), DefinitionNodeKind::Grid);
    let cells = grid
        .children()
        .map(|id| definition.node(id).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(cells.len(), 2);
    assert!(
        cells
            .iter()
            .all(|cell| cell.kind() == DefinitionNodeKind::GridCell)
    );
    assert_eq!(
        cells
            .iter()
            .map(|cell| {
                let spans = cell.grid_spans().expect("grid cells expose their spans");
                (spans.compact(), spans.wide())
            })
            .collect::<Vec<_>>(),
        [(12, 4), (6, 6)],
        "an omitted wide span must inherit its compact span"
    );
    assert_eq!(
        cells
            .iter()
            .map(|cell| {
                definition
                    .node(cell.children().next().unwrap())
                    .unwrap()
                    .binding()
                    .unwrap()
                    .as_str()
            })
            .collect::<Vec<_>>(),
        ["/first", "/second"],
        "grid cells must preserve authored document order"
    );

    for (field, value) in [
        ("compact_span", 0),
        ("compact_span", 13),
        ("wide_span", 0),
        ("wide_span", 13),
    ] {
        let document = String::from(
            r#"{
                "version": 1,
                "root": {
                    "type": "grid",
                    "value": {
                        "cells": [{
                            "compact_span": 1,
                            "wide_span": 1,
                            "child": {
                                "type": "text",
                                "value": { "content": { "fallback": "Cell" } }
                            }
                        }]
                    }
                }
            }"#,
        );
        let document = document.replacen(
            &format!(r#""{field}": 1"#),
            &format!(r#""{field}": {value}"#),
            1,
        );
        let error = parse_ui_schema_v1(document.as_bytes(), &profile)
            .expect_err("an invalid span must fail parsing");
        let (location, _) = invalid_ui_schema(error);
        assert_eq!(
            location.as_str(),
            format!("/root/value/cells/0/{field}"),
            "{field}={value} must be rejected at the span field"
        );
    }
}

#[test]
fn authored_tabs_compile_labeled_panels_in_document_order() {
    let ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "tabs",
                "value": {
                    "panels": [
                        {
                            "title": { "fallback": "Account", "key": "tabs.account" },
                            "child": {
                                "type": "control",
                                "value": {
                                    "binding": { "origin": "root", "pointer": "/name" }
                                }
                            }
                        },
                        {
                            "title": { "fallback": "Contact", "key": "tabs.contact" },
                            "child": {
                                "type": "control",
                                "value": {
                                    "binding": { "origin": "root", "pointer": "/email" }
                                }
                            }
                        }
                    ]
                }
            }
        }"#,
        &CompilationProfile::default(),
    )
    .expect("the authored tabs should parse");
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "email": { "type": "string" }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored tabs should compile");

    let tabs = definition
        .node(definition.root())
        .unwrap()
        .children()
        .next()
        .and_then(|id| definition.node(id))
        .expect("the semantic tabs should be present");
    assert_eq!(tabs.kind(), DefinitionNodeKind::Tabs);
    let panels = tabs
        .children()
        .map(|id| definition.node(id).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(panels.len(), 2);
    assert!(
        panels
            .iter()
            .all(|panel| panel.kind() == DefinitionNodeKind::TabPanel)
    );
    assert_eq!(
        panels
            .iter()
            .map(|panel| (
                panel.label(),
                panel.label_reference().unwrap().key(),
                definition
                    .node(panel.children().next().unwrap())
                    .unwrap()
                    .binding()
                    .unwrap()
                    .as_str(),
            ))
            .collect::<Vec<_>>(),
        [
            ("Account", Some("tabs.account"), "/name"),
            ("Contact", Some("tabs.contact"), "/email"),
        ]
    );
}

#[test]
fn explicit_auto_selects_and_orders_direct_properties_at_its_authored_position() {
    let ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "stack",
                "value": {
                    "children": [
                        {
                            "type": "text",
                            "value": { "content": { "fallback": "Selected fields" } }
                        },
                        {
                            "type": "auto",
                            "value": {
                                "binding": { "origin": "root", "pointer": "" },
                                "properties": {
                                    "include": ["/slash", "alpha", "beta", "hidden", "tilde~name"],
                                    "exclude": ["hidden"],
                                    "order": [
                                        { "property": "tilde~name" },
                                        "remaining",
                                        { "property": "alpha" }
                                    ]
                                }
                            }
                        }
                    ]
                }
            }
        }"#,
        &CompilationProfile::default(),
    )
    .unwrap();
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "alpha": { "type": "string" },
            "beta": { "type": "string" },
            "hidden": { "type": "string" },
            "/slash": { "type": "string" },
            "tilde~name": { "type": "string" }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the explicit Auto region should compile");

    let stack = definition
        .node(
            definition
                .node(definition.root())
                .unwrap()
                .children()
                .next()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(stack.kind(), DefinitionNodeKind::Stack);
    let children = stack
        .children()
        .map(|id| definition.node(id).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 5);
    assert_eq!(children[0].kind(), DefinitionNodeKind::Text);
    assert_eq!(
        children[1..]
            .iter()
            .map(|node| node.binding().unwrap().as_str())
            .collect::<Vec<_>>(),
        ["/tilde~0name", "/~1slash", "/beta", "/alpha"]
    );
}

#[test]
fn explicit_auto_preserves_nested_and_array_definition_identity() {
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "people": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "address": {
                            "type": "object",
                            "properties": { "city": { "type": "string" } }
                        }
                    }
                }
            },
            "profile": {
                "type": "object",
                "properties": {
                    "displayName": { "type": "string" },
                    "timezone": { "type": "string" }
                }
            },
            "omitted": { "type": "string" }
        }
    });
    let ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "auto",
                "value": {
                    "binding": { "origin": "root", "pointer": "" },
                    "properties": {
                        "include": ["people", "profile"],
                        "order": [
                            { "property": "profile" },
                            { "property": "people" }
                        ]
                    }
                }
            }
        }"#,
        &CompilationProfile::default(),
    )
    .unwrap();
    let compile = || {
        FormDefinition::compiler(data_schema.clone())
            .ui_schema(ui_schema.clone())
            .compile()
            .expect("nested generated regions should compile")
    };
    let definition = compile();
    let repeated = compile();
    assert_eq!(definition.fingerprint(), repeated.fingerprint());

    let children = definition
        .node(definition.root())
        .unwrap()
        .children()
        .collect::<Vec<_>>();
    assert_eq!(
        children
            .iter()
            .map(|id| definition.node(*id).unwrap().binding().unwrap().as_str())
            .collect::<Vec<_>>(),
        ["/profile", "/people"]
    );
    assert_eq!(
        children,
        repeated
            .node(repeated.root())
            .unwrap()
            .children()
            .collect::<Vec<_>>()
    );

    let profile = definition.node(children[0]).unwrap();
    assert_eq!(profile.semantic_kind(), Some(SemanticKind::FixedObject));
    assert_eq!(
        profile
            .children()
            .map(|id| definition.node(id).unwrap().binding().unwrap().as_str())
            .collect::<Vec<_>>(),
        ["/profile/displayName", "/profile/timezone"]
    );

    let people = definition.node(children[1]).unwrap();
    assert_eq!(people.semantic_kind(), Some(SemanticKind::HomogeneousArray));
    let template = people.children().next().expect("the array owns a template");
    let template = definition.node(template).unwrap();
    assert_eq!(template.binding().map(JsonPointer::as_str), Some(""));
    let template_children = template
        .children()
        .map(|id| definition.node(id).unwrap())
        .collect::<Vec<_>>();
    let address = template_children
        .iter()
        .find(|node| {
            node.binding()
                .is_some_and(|binding| binding.as_str() == "/address")
        })
        .expect("the nested item object should remain in the template");
    assert_eq!(
        definition
            .node(address.children().next().unwrap())
            .unwrap()
            .binding()
            .map(JsonPointer::as_str),
        Some("/address/city")
    );

    let form = definition
        .create_form(json!({
            "profile": { "displayName": "Ada", "timezone": "UTC" },
            "people": [{ "name": "Grace", "address": { "city": "Arlington" } }],
            "omitted": "preserved"
        }))
        .unwrap();
    let people = form
        .node(form.view().root())
        .unwrap()
        .children()
        .find(|id| {
            form.node(*id)
                .unwrap()
                .binding()
                .is_some_and(|binding| binding.pointer().as_str() == "/people")
        })
        .expect("the generated array should instantiate");
    let row = form
        .node(people)
        .unwrap()
        .children()
        .next()
        .expect("the array item template should instantiate once");
    assert_eq!(
        form.node(row)
            .unwrap()
            .binding()
            .unwrap()
            .pointer()
            .as_str(),
        "/people/0"
    );
}

#[test]
fn authored_scalar_array_owns_an_empty_item_template_binding() {
    let ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "/tags" },
                    "item_template": {
                        "type": "control",
                        "value": {
                            "binding": { "origin": "item_template", "pointer": "" }
                        }
                    }
                }
            }
        }"#,
        &CompilationProfile::default(),
    )
    .expect("the authored item template should parse");
    assert_eq!(
        serde_json::to_value(&ui_schema).unwrap()["root"]["value"]["item_template"]["value"]["binding"],
        json!({ "origin": "item_template", "pointer": "" }),
        "the UI-schema document must preserve its template-origin pointer"
    );

    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "tags": {
                "type": "array",
                "items": { "type": "string", "minLength": 3 }
            }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored scalar item template should compile");

    let array = definition
        .node(definition.root())
        .unwrap()
        .children()
        .next()
        .and_then(|id| definition.node(id))
        .expect("the authored array control should be present");
    assert_eq!(array.semantic_kind(), Some(SemanticKind::HomogeneousArray));
    assert_eq!(array.binding().map(JsonPointer::as_str), Some("/tags"));
    let template = array
        .children()
        .next()
        .and_then(|id| definition.node(id))
        .expect("the array should own its inline item template");
    assert_eq!(array.children().count(), 1);
    assert_eq!(template.kind(), DefinitionNodeKind::Control);
    assert_eq!(template.binding().map(JsonPointer::as_str), Some(""));

    let mut form = definition
        .form(json!({ "tags": ["same", "same"] }))
        .finding_visibility(schemaform::FindingVisibilityPolicy::new(
            schemaform::FindingVisibility::Immediate,
            schemaform::FindingVisibility::Immediate,
        ))
        .build()
        .unwrap();
    let array = form
        .node(form.view().root())
        .unwrap()
        .children()
        .next()
        .unwrap();
    let rows = form.node(array).unwrap().children().collect::<Vec<_>>();
    assert_ne!(rows[0], rows[1]);
    assert_eq!(
        form.node(rows[1])
            .unwrap()
            .binding()
            .unwrap()
            .pointer()
            .as_str(),
        "/tags/1"
    );
    form.user().input_text(rows[1], "Li").unwrap();
    assert!(
        form.node(rows[1])
            .unwrap()
            .validation_findings()
            .any(|finding| finding.code() == "minLength")
    );
    form.user().input_text(rows[1], "Paris").unwrap();
    let submission = form.prepare_submission();
    let SubmissionOutcome::Ready(snapshot) = submission.outcome() else {
        panic!("the corrected authored scalar rows should submit")
    };
    assert_eq!(snapshot.form_data(), &json!({ "tags": ["same", "Paris"] }));
}

#[test]
fn authored_fixed_object_template_edits_explicit_relative_descendants() {
    let ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "/people" },
                    "item_template": {
                        "type": "stack",
                        "value": {
                            "children": [
                                {
                                    "type": "text",
                                    "value": { "content": { "fallback": "Person details" } }
                                },
                                {
                                    "type": "control",
                                    "value": {
                                        "binding": {
                                            "origin": "item_template",
                                            "pointer": "/name"
                                        }
                                    }
                                },
                                {
                                    "type": "group",
                                    "value": {
                                        "title": { "fallback": "Location" },
                                        "child": {
                                            "type": "control",
                                            "value": {
                                                "binding": {
                                                    "origin": "item_template",
                                                    "pointer": "/address/city"
                                                }
                                            }
                                        }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        }"#,
        &CompilationProfile::default(),
    )
    .unwrap();
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["people"],
        "properties": {
            "people": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["name", "address"],
                    "properties": {
                        "name": { "type": "string", "minLength": 3 },
                        "address": {
                            "type": "object",
                            "required": ["city"],
                            "properties": {
                                "city": { "type": "string", "minLength": 3 }
                            }
                        }
                    }
                }
            }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored fixed-object item template should compile");

    let array = definition
        .node(definition.root())
        .unwrap()
        .children()
        .next()
        .unwrap();
    let template = definition.node(array).unwrap().children().next().unwrap();
    assert_eq!(
        definition.node(template).unwrap().kind(),
        DefinitionNodeKind::AutoGeneratedLayout
    );
    assert_eq!(
        definition
            .node(template)
            .unwrap()
            .binding()
            .map(JsonPointer::as_str),
        Some("")
    );
    let authored_stack = definition
        .node(template)
        .unwrap()
        .children()
        .next()
        .unwrap();
    assert_eq!(
        definition.node(authored_stack).unwrap().kind(),
        DefinitionNodeKind::Stack
    );
    let mut template_bindings = Vec::new();
    for id in definition.node(authored_stack).unwrap().children() {
        let node = definition.node(id).unwrap();
        if let Some(binding) = node.binding() {
            template_bindings.push(binding.as_str().to_owned());
        }
        template_bindings.extend(node.children().filter_map(|child| {
            definition
                .node(child)
                .unwrap()
                .binding()
                .map(|binding| binding.as_str().to_owned())
        }));
    }
    assert_eq!(template_bindings, ["/name", "/address/city"]);
    assert!(
        template_bindings
            .iter()
            .all(|binding| !binding.contains("people"))
    );

    let duplicate = json!({ "name": "Ada", "address": { "city": "Rome" } });
    let mut form = definition
        .form(json!({ "people": [duplicate.clone(), duplicate] }))
        .finding_visibility(schemaform::FindingVisibilityPolicy::new(
            schemaform::FindingVisibility::Immediate,
            schemaform::FindingVisibility::Immediate,
        ))
        .build()
        .unwrap();
    let array = form
        .node(form.view().root())
        .unwrap()
        .children()
        .next()
        .unwrap();
    let rows = form.node(array).unwrap().children().collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    assert_ne!(rows[0], rows[1]);
    assert_eq!(
        form.node(rows[0])
            .unwrap()
            .binding()
            .unwrap()
            .pointer()
            .as_str(),
        "/people/0"
    );
    assert_ne!(
        form.node(rows[0]).unwrap().item_identity(),
        form.node(rows[1]).unwrap().item_identity()
    );
    let second_city = descendant_with_binding(&form, rows[1], "/people/1/address/city");

    form.user().input_text(second_city, "Li").unwrap();
    assert!(
        form.node(second_city)
            .unwrap()
            .validation_findings()
            .any(|finding| finding.code() == "minLength")
    );
    form.user().input_text(second_city, "Paris").unwrap();
    let submission = form.prepare_submission();
    let SubmissionOutcome::Ready(snapshot) = submission.outcome() else {
        panic!("the corrected authored rows should submit")
    };
    assert_eq!(
        snapshot.form_data(),
        &json!({
            "people": [
                { "name": "Ada", "address": { "city": "Rome" } },
                { "name": "Ada", "address": { "city": "Paris" } }
            ]
        })
    );
}

#[test]
fn authored_fixed_object_template_root_owns_item_authority_findings_and_repair() {
    let ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "/people" },
                    "item_template": {
                        "type": "stack",
                        "value": {
                            "children": [
                                {
                                    "type": "control",
                                    "value": {
                                        "binding": {
                                            "origin": "item_template",
                                            "pointer": "/name"
                                        }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        }"#,
        &CompilationProfile::default(),
    )
    .unwrap();
    let compile = |read_only| {
        FormDefinition::compiler(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "people": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "readOnly": read_only,
                        "required": ["name"],
                        "properties": { "name": { "type": "string" } }
                    }
                }
            }
        }))
        .ui_schema(ui_schema.clone())
        .compile()
        .unwrap()
    };

    let read_only = compile(true);
    let mut form = read_only
        .create_form(json!({ "people": [{ "name": "Ada" }] }))
        .unwrap();
    let array = form
        .node(form.view().root())
        .unwrap()
        .children()
        .next()
        .unwrap();
    let row = form.node(array).unwrap().children().next().unwrap();
    assert_eq!(
        form.node(row)
            .unwrap()
            .binding()
            .unwrap()
            .pointer()
            .as_str(),
        "/people/0"
    );
    let name = descendant_with_binding(&form, row, "/people/0/name");
    assert!(form.node(name).unwrap().is_read_only());
    assert!(form.user().input_text(name, "Grace").is_err());
    assert_eq!(form.form_data(), &json!({ "people": [{ "name": "Ada" }] }));

    let editable = compile(false);
    let form = editable
        .form(json!({ "people": [{}] }))
        .finding_visibility(schemaform::FindingVisibilityPolicy::new(
            schemaform::FindingVisibility::Immediate,
            schemaform::FindingVisibility::Immediate,
        ))
        .build()
        .unwrap();
    let array = form
        .node(form.view().root())
        .unwrap()
        .children()
        .next()
        .unwrap();
    let row = form.node(array).unwrap().children().next().unwrap();
    assert!(
        form.node(row)
            .unwrap()
            .validation_findings()
            .any(|finding| finding.code() == "required")
    );

    let mut incompatible = editable.create_form(json!({ "people": [7] })).unwrap();
    let array = incompatible
        .node(incompatible.view().root())
        .unwrap()
        .children()
        .next()
        .unwrap();
    let row = incompatible.node(array).unwrap().children().next().unwrap();
    let row_view = incompatible.node(row).unwrap();
    assert!(row_view.allowed_operations().can_replace_value());
    let seed = row_view.definition().creation_seed().cloned().unwrap();
    incompatible.user().replace_value(row, seed).unwrap();
    assert_eq!(incompatible.form_data(), &json!({ "people": [{}] }));
}

#[test]
fn authored_item_auto_materializes_only_its_template_relative_definition() {
    let ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "/people" },
                    "item_template": {
                        "type": "auto",
                        "value": {
                            "binding": { "origin": "item_template", "pointer": "" },
                            "properties": {
                                "include": ["name", "city"],
                                "order": [
                                    { "property": "city" },
                                    { "property": "name" }
                                ]
                            }
                        }
                    }
                }
            }
        }"#,
        &CompilationProfile::default(),
    )
    .unwrap();
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "people": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "city": { "type": "string" }
                    }
                }
            },
            "city": { "type": "string" }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the item-relative Auto should compile");

    let array = definition
        .node(definition.root())
        .unwrap()
        .children()
        .next()
        .unwrap();
    let template = definition
        .node(array)
        .unwrap()
        .children()
        .next()
        .and_then(|id| definition.node(id))
        .expect("the array should own one generated template root");
    assert_eq!(template.binding().map(JsonPointer::as_str), Some(""));
    assert_eq!(
        template
            .children()
            .map(|id| definition.node(id).unwrap().binding().unwrap().as_str())
            .collect::<Vec<_>>(),
        ["/city", "/name"]
    );
}

#[test]
fn authored_item_template_duplicate_checks_are_scoped_per_array() {
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "first": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "name": { "type": "string" } }
                }
            },
            "second": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": { "name": { "type": "string" } }
                }
            }
        }
    });
    let compile = |document: &[u8]| {
        FormDefinition::compiler(data_schema.clone())
            .ui_schema(parse_ui_schema_v1(document, &CompilationProfile::default()).unwrap())
            .compile()
    };

    compile(
        br#"{
            "version": 1,
            "root": {
                "type": "stack",
                "value": {
                    "children": [
                        {
                            "type": "control",
                            "value": {
                                "binding": { "origin": "root", "pointer": "/first" },
                                "item_template": {
                                    "type": "control",
                                    "value": {
                                        "binding": {
                                            "origin": "item_template",
                                            "pointer": "/name"
                                        }
                                    }
                                }
                            }
                        },
                        {
                            "type": "control",
                            "value": {
                                "binding": { "origin": "root", "pointer": "/second" },
                                "item_template": {
                                    "type": "control",
                                    "value": {
                                        "binding": {
                                            "origin": "item_template",
                                            "pointer": "/name"
                                        }
                                    }
                                }
                            }
                        }
                    ]
                }
            }
        }"#,
    )
    .expect("the same relative binding belongs to separate instantiated domains");

    let error = match compile(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "/first" },
                    "item_template": {
                        "type": "stack",
                        "value": {
                            "children": [
                                {
                                    "type": "control",
                                    "value": {
                                        "binding": {
                                            "origin": "item_template",
                                            "pointer": "/name"
                                        }
                                    }
                                },
                                {
                                    "type": "control",
                                    "value": {
                                        "binding": {
                                            "origin": "item_template",
                                            "pointer": "/name"
                                        }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        }"#,
    ) {
        Err(error) => error,
        Ok(_) => panic!("one item template cannot edit the same relative binding twice"),
    };
    assert_ui_input_error(
        error,
        "/root/value/item_template/value/children/1/value/binding/pointer",
        UiSchemaInputErrorKind::DuplicateBinding,
    );

    let error = match compile(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "root", "pointer": "/first" },
                    "item_template": {
                        "type": "stack",
                        "value": {
                            "children": [
                                {
                                    "type": "control",
                                    "value": {
                                        "binding": {
                                            "origin": "item_template",
                                            "pointer": "/name"
                                        }
                                    }
                                },
                                {
                                    "type": "auto",
                                    "value": {
                                        "binding": {
                                            "origin": "item_template",
                                            "pointer": ""
                                        },
                                        "properties": { "include": ["name"] }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        }"#,
    ) {
        Err(error) => error,
        Ok(_) => panic!("an item Auto cannot overlap an explicit editor"),
    };
    assert_ui_input_error(
        error,
        "/root/value/item_template/value/children/1/value/binding/pointer",
        UiSchemaInputErrorKind::DuplicateBinding,
    );
}

#[test]
fn omitted_ui_schema_matches_an_explicit_root_auto() {
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "/slash": { "type": "string" },
            "alpha": { "type": "string" },
            "name": { "type": "string" },
            "address": {
                "type": "object",
                "properties": {
                    "/line": { "type": "string" },
                    "city": { "type": "string" }
                }
            }
        }
    });
    let generated = FormDefinition::compile(data_schema.clone()).unwrap();
    let explicit = FormDefinition::compiler(data_schema)
        .ui_schema(
            parse_ui_schema_v1(
                br#"{
                    "version": 1,
                    "root": {
                        "type": "auto",
                        "value": { "binding": { "origin": "root", "pointer": "" } }
                    }
                }"#,
                &CompilationProfile::default(),
            )
            .unwrap(),
        )
        .compile()
        .unwrap();

    assert_eq!(generated.fingerprint(), explicit.fingerprint());
    let generated_children = generated
        .node(generated.root())
        .unwrap()
        .children()
        .collect::<Vec<_>>();
    assert_eq!(
        generated_children,
        explicit
            .node(explicit.root())
            .unwrap()
            .children()
            .collect::<Vec<_>>()
    );
    assert_eq!(
        generated_children
            .iter()
            .map(|id| generated.node(*id).unwrap().binding().unwrap().as_str())
            .collect::<Vec<_>>(),
        ["/~1slash", "/address", "/alpha", "/name"]
    );
    let address = generated.node(*generated_children.get(1).unwrap()).unwrap();
    assert_eq!(
        address
            .children()
            .map(|id| generated.node(id).unwrap().binding().unwrap().as_str())
            .collect::<Vec<_>>(),
        ["/address/~1line", "/address/city"]
    );
}

#[test]
fn auto_only_materializes_selected_unsupported_regions() {
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "contact": {
                "oneOf": [{ "type": "string" }, { "type": "integer" }]
            },
            "name": { "type": "string" }
        }
    });
    let analyze = |included: &str| {
        let document = format!(
            r#"{{
                "version": 1,
                "root": {{
                    "type": "auto",
                    "value": {{
                        "binding": {{ "origin": "root", "pointer": "" }},
                        "properties": {{ "include": ["{included}"] }}
                    }}
                }}
            }}"#
        );
        FormDefinition::compiler(data_schema.clone())
            .ui_schema(
                parse_ui_schema_v1(document.as_bytes(), &CompilationProfile::default()).unwrap(),
            )
            .analyze()
            .expect("lenient analysis should retain the capability report")
    };

    let omitted = analyze("name");
    assert!(omitted.capability_report().findings().next().is_some());
    assert!(
        omitted
            .definition()
            .node(omitted.definition().root())
            .unwrap()
            .children()
            .all(|id| omitted.definition().node(id).unwrap().kind()
                != DefinitionNodeKind::Unsupported)
    );

    let selected = analyze("contact");
    assert_eq!(selected.capability_report(), omitted.capability_report());
    let contact = selected
        .definition()
        .node(selected.definition().root())
        .unwrap()
        .children()
        .next()
        .and_then(|id| selected.definition().node(id))
        .expect("the selected unsupported region should be explicit");
    assert_eq!(contact.kind(), DefinitionNodeKind::Unsupported);
    assert_eq!(contact.binding().map(JsonPointer::as_str), Some("/contact"));

    let scoped = FormDefinition::compiler(data_schema)
        .ui_schema(
            parse_ui_schema_v1(
                br#"{
                    "version": 1,
                    "root": {
                        "type": "auto",
                        "value": {
                            "binding": { "origin": "root", "pointer": "/contact" }
                        }
                    }
                }"#,
                &CompilationProfile::default(),
            )
            .unwrap(),
        )
        .analyze()
        .expect("a scoped Auto should retain its unsupported region");
    let contact = scoped
        .definition()
        .node(scoped.definition().root())
        .unwrap()
        .children()
        .next()
        .and_then(|id| scoped.definition().node(id))
        .expect("the scoped unsupported region should be explicit");
    assert_eq!(contact.kind(), DefinitionNodeKind::Unsupported);
    assert_eq!(contact.binding().map(JsonPointer::as_str), Some("/contact"));

    let invalid_ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "auto",
                "value": {
                    "binding": { "origin": "root", "pointer": "/contact" },
                    "properties": { "order": ["remaining", "remaining"] }
                }
            }
        }"#,
        &CompilationProfile::default(),
    )
    .unwrap();
    let error = match FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "contact": {
                "oneOf": [{ "type": "string" }, { "type": "integer" }]
            }
        }
    }))
    .ui_schema(invalid_ui_schema)
    .compile()
    {
        Err(error) => error,
        Ok(_) => panic!("an invalid scoped property order must not compile"),
    };
    assert_ui_input_error(
        error,
        "/root/value/properties/order/1",
        UiSchemaInputErrorKind::InvalidPropertySelection,
    );

    let root_blocker_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "kind": { "type": "string" },
            "name": { "type": "string" }
        },
        "if": { "properties": { "kind": { "const": "business" } } },
        "then": { "properties": { "taxId": { "type": "string" } } }
    });
    let selective_root = FormDefinition::compiler(root_blocker_schema)
        .ui_schema(
            parse_ui_schema_v1(
                br#"{
                    "version": 1,
                    "root": {
                        "type": "auto",
                        "value": {
                            "binding": { "origin": "root", "pointer": "" },
                            "properties": { "include": ["name"] }
                        }
                    }
                }"#,
                &CompilationProfile::default(),
            )
            .unwrap(),
        )
        .analyze()
        .expect("a selective root Auto should retain root capability blockers");
    let root_children = selective_root
        .definition()
        .node(selective_root.definition().root())
        .unwrap()
        .children()
        .map(|id| selective_root.definition().node(id).unwrap())
        .collect::<Vec<_>>();
    assert!(root_children.iter().any(|node| {
        node.kind() == DefinitionNodeKind::Unsupported
            && node.binding().map(JsonPointer::as_str) == Some("")
    }));
    assert!(root_children.iter().any(|node| {
        node.kind() == DefinitionNodeKind::Control
            && node.binding().map(JsonPointer::as_str) == Some("/name")
    }));
    assert!(
        root_children
            .iter()
            .all(|node| { node.binding().map(JsonPointer::as_str) != Some("/kind") })
    );
}

#[test]
fn authored_ui_schema_rejects_invalid_and_duplicate_bindings_with_locations() {
    let data_schema = json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "profile": {
                "type": "object",
                "properties": {
                    "first": { "type": "string" },
                    "second": { "type": "string" }
                }
            }
        }
    });
    let compile = |document: &[u8]| {
        let ui_schema = parse_ui_schema_v1(document, &CompilationProfile::default()).unwrap();
        match FormDefinition::compiler(data_schema.clone())
            .default_dialect(Dialect::Draft202012)
            .ui_schema(ui_schema)
            .compile()
        {
            Ok(_) => panic!("the invalid UI schema must not compile"),
            Err(error) => error,
        }
    };

    let error = compile(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": { "binding": { "origin": "root", "pointer": "/missing" } }
            }
        }"#,
    );
    assert_ui_input_error(
        error,
        "/root/value/binding/pointer",
        UiSchemaInputErrorKind::UnknownBinding,
    );

    let error = compile(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": {
                    "binding": { "origin": "item_template", "pointer": "/name" }
                }
            }
        }"#,
    );
    assert_ui_input_error(
        error,
        "/root/value/binding/origin",
        UiSchemaInputErrorKind::InvalidBindingOrigin,
    );

    let error = compile(
        br#"{
            "version": 1,
            "root": {
                "type": "stack",
                "value": {
                    "children": [
                        {
                            "type": "control",
                            "value": { "binding": { "origin": "root", "pointer": "/name" } }
                        },
                        {
                            "type": "control",
                            "value": { "binding": { "origin": "root", "pointer": "/name" } }
                        }
                    ]
                }
            }
        }"#,
    );
    assert_ui_input_error(
        error,
        "/root/value/children/1/value/binding/pointer",
        UiSchemaInputErrorKind::DuplicateBinding,
    );

    let error = compile(
        br#"{
            "version": 1,
            "root": {
                "type": "stack",
                "value": {
                    "children": [
                        {
                            "type": "auto",
                            "value": {
                                "binding": { "origin": "root", "pointer": "/profile" },
                                "properties": { "include": ["first"] }
                            }
                        },
                        {
                            "type": "auto",
                            "value": {
                                "binding": { "origin": "root", "pointer": "/profile" },
                                "properties": { "include": ["second"] }
                            }
                        }
                    ]
                }
            }
        }"#,
    );
    assert_ui_input_error(
        error,
        "/root/value/children/1/value/binding/pointer",
        UiSchemaInputErrorKind::DuplicateBinding,
    );

    let error = compile(
        br#"{
            "version": 1,
            "root": {
                "type": "stack",
                "value": {
                    "children": [
                        {
                            "type": "control",
                            "value": { "binding": { "origin": "root", "pointer": "/name" } }
                        },
                        {
                            "type": "auto",
                            "value": { "binding": { "origin": "root", "pointer": "" } }
                        }
                    ]
                }
            }
        }"#,
    );
    assert_ui_input_error(
        error,
        "/root/value/children/1/value/binding/pointer",
        UiSchemaInputErrorKind::DuplicateBinding,
    );

    let error = compile(
        br#"{
            "version": 1,
            "root": {
                "type": "auto",
                "value": {
                    "binding": { "origin": "root", "pointer": "" },
                    "properties": { "include": ["missing"] }
                }
            }
        }"#,
    );
    assert_ui_input_error(
        error,
        "/root/value/properties/include/0",
        UiSchemaInputErrorKind::UnknownBinding,
    );

    let error = compile(
        br#"{
            "version": 1,
            "root": {
                "type": "auto",
                "value": {
                    "binding": { "origin": "root", "pointer": "" },
                    "properties": { "order": ["remaining", "remaining"] }
                }
            }
        }"#,
    );
    assert_ui_input_error(
        error,
        "/root/value/properties/order/1",
        UiSchemaInputErrorKind::InvalidPropertySelection,
    );
}

fn assert_ui_input_error(
    error: CompileError,
    expected_location: &str,
    expected_kind: UiSchemaInputErrorKind,
) {
    let CompileError::Input(InputError::InvalidUiSchema(error)) = error else {
        panic!("expected a UI-schema input error, got {error:?}");
    };
    assert_eq!(error.location().as_str(), expected_location);
    assert_eq!(error.kind(), expected_kind);
}

fn descendant_with_binding(
    form: &schemaform::Form,
    root: schemaform::InstanceIdentity,
    binding: &str,
) -> schemaform::InstanceIdentity {
    let mut pending = vec![root];
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).unwrap();
        if node
            .binding()
            .is_some_and(|current| current.pointer().as_str() == binding)
        {
            return identity;
        }
        pending.extend(node.children());
    }
    panic!("missing form node for {binding}")
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
