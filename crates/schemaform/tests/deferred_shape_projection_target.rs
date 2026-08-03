use schemaform::{
    CompileError, FormDefinition, JsonPointer, RetrievalUri, SchemaResource, SubmissionOutcome,
    definition::DefinitionNodeKind,
    form::{SubmissionBlocker, ValidationOutcomeView},
};
use serde_json::{Value, json};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const ROOT_URI: &str = "urn:schemaform:test:deferred-shape-projection";

fn compiler(schema: Value) -> schemaform::FormCompiler {
    FormDefinition::compiler(schema)
        .root_uri(RetrievalUri::parse(ROOT_URI).expect("the fixture retrieval URI should be valid"))
}

fn assert_strict_and_lenient_finding(
    schema: Value,
    code: &str,
    instance_location: &str,
    keyword_location: &str,
) {
    let strict_report = match compiler(schema.clone()).compile() {
        Err(CompileError::Capability(report)) => report,
        Err(error) => panic!("strict compilation returned the wrong error: {error}"),
        Ok(_) => panic!("strict compilation must reject the deferred shape"),
    };
    let analysis = compiler(schema.clone())
        .analyze()
        .expect("lenient analysis should retain an explicit unsupported region");
    let repeated = compiler(schema)
        .analyze()
        .expect("repeated lenient analysis should remain deterministic");

    assert_eq!(&strict_report, analysis.capability_report());
    assert_eq!(analysis.capability_report(), repeated.capability_report());
    assert_eq!(
        analysis.definition().fingerprint(),
        repeated.definition().fingerprint()
    );
    let matching = strict_report
        .findings()
        .filter(|finding| finding.code() == code)
        .collect::<Vec<_>>();
    assert_eq!(matching.len(), 1);
    assert_eq!(matching[0].instance_location().as_str(), instance_location);
    assert_eq!(matching[0].keyword_location().resource().as_str(), ROOT_URI);
    assert_eq!(
        matching[0].keyword_location().pointer().as_str(),
        keyword_location
    );
    assert!(matching[0].is_blocking());

    let unsupported = analysis
        .definition()
        .node(analysis.definition().root())
        .expect("the definition root should exist")
        .children()
        .find_map(|identity| {
            let node = analysis.definition().node(identity)?;
            (node.binding().map(JsonPointer::as_str) == Some(instance_location)).then_some(node)
        })
        .expect("the deferred shape should have one explicit definition node");
    assert_eq!(unsupported.kind(), DefinitionNodeKind::Unsupported);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn unevaluated_items_blocks_when_it_determines_the_item_shape() {
    assert_strict_and_lenient_finding(
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "values": {
                    "type": "array",
                    "unevaluatedItems": { "type": "string" }
                }
            }
        }),
        "unevaluated.items.shape",
        "/values",
        "/properties/values/unevaluatedItems",
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn recursive_projection_stops_once_and_keeps_recursive_validation_authoritative() {
    let schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": { "type": "string" },
            "child": {
                "title": "Child",
                "$ref": "#"
            }
        }
    });
    assert_strict_and_lenient_finding(
        schema.clone(),
        "structure.recursive.projection",
        "/child",
        "/properties/child/$ref",
    );

    let analysis = compiler(schema)
        .analyze()
        .expect("the legal recursive graph should remain analyzable");
    assert_eq!(
        analysis
            .capability_report()
            .findings()
            .filter(|finding| finding.is_blocking())
            .count(),
        1
    );
    let definition = analysis.into_parts().0;
    let mut form = definition
        .create_form(json!({ "name": "root", "child": { "name": "nested" } }))
        .expect("recursive form data should instantiate the finite definition");
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );

    form.transact(|draft| {
        draft.set(
            &JsonPointer::parse("/child/name").expect("the fixture pointer should be valid"),
            json!(7),
        );
    })
    .expect("the host should be able to install invalid recursive form data");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| {
                finding.code() == "type"
                    && finding.instance_location().as_str() == "/child/name"
            })
    ));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
                && blockers.iter().any(|blocker| matches!(
                    blocker,
                    SubmissionBlocker::Capability(finding)
                        if finding.code() == "structure.recursive.projection"
                ))
    ));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn reusing_one_object_schema_on_sibling_paths_is_not_recursive() {
    let definition = compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "billing": { "$ref": "#/$defs/contact" },
            "shipping": { "$ref": "#/$defs/contact" }
        },
        "$defs": {
            "contact": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                }
            }
        }
    }))
    .compile()
    .expect("schema reuse on separate projection paths should compile");
    let root = definition
        .node(definition.root())
        .expect("the definition root should exist");
    let mut bindings = Vec::new();
    for object in root.children() {
        let object = definition
            .node(object)
            .expect("each root child should remain addressable");
        for control in object.children() {
            let control = definition
                .node(control)
                .expect("each object child should remain addressable");
            if let Some(binding) = control.binding() {
                bindings.push(binding.as_str());
            }
        }
    }

    assert!(bindings.contains(&"/billing/name"));
    assert!(bindings.contains(&"/shipping/name"));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn dynamic_reference_blocks_only_when_it_determines_the_projected_shape() {
    assert_strict_and_lenient_finding(
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$dynamicAnchor": "node",
            "type": "object",
            "properties": {
                "dynamic": { "$dynamicRef": "#node" }
            }
        }),
        "core.dynamic-reference.shape",
        "/dynamic",
        "/properties/dynamic/$dynamicRef",
    );

    let definition = compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "$dynamicRef": "#non-empty"
            }
        },
        "$defs": {
            "non-empty": {
                "$dynamicAnchor": "non-empty",
                "minLength": 1
            }
        }
    }))
    .compile()
    .expect("an independently known string shape should retain dynamic validation");
    let form = definition
        .create_form(json!({ "name": "" }))
        .expect("the dynamic-reference form should instantiate");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "minLength")
    ));

    assert_strict_and_lenient_finding(
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "profile": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" }
                    },
                    "$dynamicRef": "#profile-shape"
                }
            },
            "$defs": {
                "profile-shape": {
                    "$dynamicAnchor": "profile-shape",
                    "properties": {
                        "dynamicField": { "type": "string" }
                    }
                }
            }
        }),
        "core.dynamic-reference.shape",
        "/profile",
        "/properties/profile/$dynamicRef",
    );

    let definition = compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "profile": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "$dynamicRef": "#profile-rules"
            }
        },
        "$defs": {
            "profile-rules": {
                "$dynamicAnchor": "profile-rules",
                "minProperties": 1
            }
        }
    }))
    .compile()
    .expect("dynamic object constraints should remain validation-only");
    let form = definition
        .create_form(json!({ "profile": {} }))
        .expect("the dynamic object-constraint form should instantiate");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "minProperties")
    ));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn dynamic_validation_resources_participate_in_the_definition_fingerprint() {
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "$dynamicRef": "https://schemas.example/rules.json#value"
            }
        }
    });
    let compile = |minimum_length| {
        FormDefinition::compiler(root.clone())
            .root_uri(
                RetrievalUri::parse(ROOT_URI).expect("the fixture retrieval URI should be valid"),
            )
            .resource(SchemaResource::new(
                RetrievalUri::parse("https://schemas.example/rules.json")
                    .expect("the dynamic resource URI should be valid"),
                json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "$dynamicAnchor": "value",
                    "$ref": "https://schemas.example/string-rules.json"
                }),
            ))
            .resource(SchemaResource::new(
                RetrievalUri::parse("https://schemas.example/string-rules.json")
                    .expect("the transitive resource URI should be valid"),
                json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "minLength": minimum_length
                }),
            ))
            .compile()
            .expect("the independently shaped dynamic reference should compile")
    };

    assert_ne!(compile(1).fingerprint(), compile(2).fingerprint());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn validation_only_reference_resources_participate_in_the_definition_fingerprint() {
    let root = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "not": { "$ref": "https://schemas.example/rules.json" }
            }
        }
    });
    let compile = |forbidden| {
        FormDefinition::compiler(root.clone())
            .root_uri(
                RetrievalUri::parse(ROOT_URI).expect("the fixture retrieval URI should be valid"),
            )
            .resource(SchemaResource::new(
                RetrievalUri::parse("https://schemas.example/rules.json")
                    .expect("the validation resource URI should be valid"),
                json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "const": forbidden
                }),
            ))
            .compile()
            .expect("the validation-only reference should compile")
    };

    assert_ne!(compile("Ada").fingerprint(), compile("Grace").fingerprint());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn unions_and_tuples_have_construct_specific_unsupported_findings() {
    assert_strict_and_lenient_finding(
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "choice": {
                    "type": "string",
                    "anyOf": [
                        { "minLength": 2 },
                        { "pattern": "^[A-Z]" }
                    ]
                }
            }
        }),
        "applicator.any-of",
        "/choice",
        "/properties/choice/anyOf",
    );
    assert_strict_and_lenient_finding(
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "tuple": {
                    "type": "array",
                    "prefixItems": [
                        { "type": "string" },
                        { "type": "integer" }
                    ]
                }
            }
        }),
        "applicator.prefix-items",
        "/tuple",
        "/properties/tuple/prefixItems",
    );

    compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "text": {
                "type": "string",
                "prefixItems": [{ "type": "integer" }]
            }
        }
    }))
    .compile()
    .expect("an inapplicable tuple keyword should not block a string control");

    let analysis = compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "flag": {
                "type": "boolean",
                "prefixItems": [{ "type": "integer" }]
            }
        }
    }))
    .analyze()
    .expect("an unsupported boolean should remain analyzable");
    assert!(
        analysis
            .capability_report()
            .findings()
            .all(|finding| finding.code() != "applicator.prefix-items")
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn structural_conditionals_block_without_rejecting_validation_only_conditionals() {
    let structural = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "profile": {
                "type": "object",
                "properties": {
                    "kind": { "type": "string" }
                },
                "if": {
                    "properties": { "kind": { "const": "business" } }
                },
                "then": {
                    "required": ["taxId"],
                    "properties": { "taxId": { "type": "string" } }
                },
                "else": {
                    "properties": { "nickname": { "type": "string" } }
                }
            }
        }
    });
    for (code, keyword) in [
        ("applicator.if.structural", "if"),
        ("applicator.then.structural", "then"),
        ("applicator.else.structural", "else"),
    ] {
        assert_strict_and_lenient_finding(
            structural.clone(),
            code,
            "/profile",
            &format!("/properties/profile/{keyword}"),
        );
    }

    assert_strict_and_lenient_finding(
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "profile": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string" },
                        "value": { "type": "string" }
                    },
                    "if": {
                        "properties": { "kind": { "const": "numeric" } }
                    },
                    "then": {
                        "properties": { "value": { "type": "integer" } }
                    }
                }
            }
        }),
        "applicator.then.structural",
        "/profile",
        "/properties/profile/then",
    );
    assert_strict_and_lenient_finding(
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "profile": {
                    "type": "object",
                    "properties": {
                        "first": { "$ref": "#/$defs/integer" },
                        "kind": { "type": "string" },
                        "second": { "$ref": "#/$defs/string" }
                    },
                    "if": {
                        "properties": { "kind": { "const": "numeric" } }
                    },
                    "then": {
                        "properties": {
                            "first": { "$ref": "#/$defs/integer" },
                            "second": { "$ref": "#/$defs/integer" }
                        }
                    }
                }
            },
            "$defs": {
                "integer": { "type": "integer" },
                "string": { "type": "string" }
            }
        }),
        "applicator.then.structural",
        "/profile",
        "/properties/profile/then",
    );
    assert_strict_and_lenient_finding(
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "profile": {
                    "type": "object",
                    "properties": {
                        "kind": { "type": "string" }
                    },
                    "if": {
                        "properties": { "kind": { "const": "business" } }
                    },
                    "then": {
                        "$id": "https://schemas.example/conditions/then",
                        "$ref": "shape"
                    }
                }
            },
            "$defs": {
                "business": {
                    "$id": "https://schemas.example/conditions/shape",
                    "properties": { "taxId": { "type": "string" } }
                }
            }
        }),
        "applicator.then.structural",
        "/profile",
        "/properties/profile/then",
    );

    compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "profile": {
                "type": "object",
                "properties": {
                    "name": { "type": "string" }
                },
                "then": {
                    "properties": { "ignored": { "type": "string" } }
                }
            }
        }
    }))
    .compile()
    .expect("then without if should not affect validation or projection");

    compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "profile": {
                "type": "object",
                "required": ["name"],
                "properties": {
                    "kind": { "type": "string" },
                    "name": { "$ref": "#/$defs/text" }
                },
                "if": {
                    "properties": { "kind": { "const": "named" } }
                },
                "then": {
                    "required": ["name"],
                    "properties": {
                        "name": { "type": "string" }
                    }
                }
            }
        },
        "$defs": {
            "text": { "type": "string" }
        }
    }))
    .compile()
    .expect("redundant requiredness and kinds should remain validation-only");

    let definition = compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "code": {
                "type": "string",
                "if": { "pattern": "^[A-Z]" },
                "then": { "minLength": 3 },
                "else": { "maxLength": 1 }
            }
        }
    }))
    .compile()
    .expect("constraint-only conditionals should remain validator-authoritative");
    let form = definition
        .create_form(json!({ "code": "AB" }))
        .expect("the validation-only conditional form should instantiate");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "minLength")
    ));
    let form = definition
        .create_form(json!({ "code": "ab" }))
        .expect("the validation-only else branch should instantiate");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "maxLength")
    ));

    let definition = compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "code": {
                "type": "string",
                "if": { "minLength": 1 },
                "then": { "not": { "enum": ["blocked"] } }
            }
        }
    }))
    .compile()
    .expect("an enum below a validation-only applicator should not change the projected choice");
    let form = definition
        .create_form(json!({ "code": "blocked" }))
        .expect("the validation-only enum form should instantiate");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "not")
    ));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn validation_only_applicators_keep_an_independently_known_projection() {
    let definition = compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$comment": "Accepted validation-only keywords through the public product path.",
        "type": "object",
        "additionalProperties": false,
        "minProperties": 1,
        "maxProperties": 4,
        "dependentRequired": { "profile": ["text"] },
        "propertyNames": { "pattern": "^[a-z]+$" },
        "unevaluatedProperties": false,
        "properties": {
            "text": {
                "type": "string",
                "minLength": 1,
                "maxLength": 8,
                "pattern": "^[a-z]+$",
                "not": { "const": "blocked" }
            },
            "amount": {
                "type": "number",
                "maximum": 10,
                "exclusiveMinimum": 0,
                "exclusiveMaximum": 11,
                "multipleOf": 0.5
            },
            "true_schema": {
                "allOf": [true, { "type": "string" }]
            },
            "extra": { "type": "string" },
            "profile": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "kind": { "type": "string" },
                    "detail": { "type": "string" }
                },
                "if": {
                    "properties": { "kind": { "enum": ["business"] } }
                },
                "then": {
                    "properties": { "detail": { "type": "string", "minLength": 2 } }
                },
                "else": {
                    "properties": { "detail": { "type": "string", "maxLength": 5 } }
                },
                "dependentSchemas": {
                    "kind": {
                        "properties": { "detail": { "type": "string", "pattern": "^[A-Z]" } }
                    }
                },
                "unevaluatedProperties": false
            },
            "values": {
                "type": "array",
                "items": { "type": "string" },
                "unevaluatedItems": false
            }
        }
    }))
    .compile()
    .expect("validation-only applicators should preserve independent controls");

    assert!(
        definition
            .node(definition.root())
            .expect("the definition root should exist")
            .children()
            .filter_map(|identity| definition.node(identity))
            .any(|node| node
                .binding()
                .is_some_and(|binding| binding.as_str() == "/true_schema")),
        "a true boolean schema must preserve an independently supplied string control"
    );

    let form = definition
        .create_form(json!({
            "text": "blocked",
            "amount": 12.25,
            "profile": { "kind": "business", "detail": "x" },
            "values": ["preserved"]
        }))
        .expect("invalid validation-only data should remain constructible");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if ["not", "maximum", "exclusiveMaximum", "multipleOf", "minLength", "pattern"]
                .into_iter()
                .all(|code| {
                findings.iter().any(|finding| finding.code() == code)
            })
    ));

    let form = definition
        .create_form(json!({ "text": "ok", "amount": 0 }))
        .expect("an exclusive-minimum violation should remain constructible");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "exclusiveMinimum")
    ));

    let form = definition
        .create_form(json!({ "profile": { "kind": "personal" } }))
        .expect("a dependent-required violation should remain constructible");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| {
                finding.code() == "required"
                    && finding
                        .keyword_location()
                        .pointer()
                        .as_str()
                        .contains("/dependentRequired/")
            })
    ));

    for (form_data, code) in [
        (json!({}), "minProperties"),
        (
            json!({
                "text": "ok",
                "amount": 1,
                "extra": "present",
                "profile": {},
                "values": []
            }),
            "maxProperties",
        ),
        (json!({ "Invalid": true }), "propertyNames"),
        (json!({ "text": "toolongvalue" }), "maxLength"),
    ] {
        let form = definition
            .create_form(form_data)
            .expect("validation-only violations should remain constructible");
        assert!(matches!(
            form.view().validation_outcome(),
            ValidationOutcomeView::Invalid { findings, .. }
                if findings.iter().any(|finding| finding.code() == code)
        ));
    }
}
