use schemaform::{
    FindingView, FormDefinition, RetrievalUri, SubmissionOutcome,
    definition::SemanticKind,
    form::{ParseBlockerKind, ScalarValueState, SubmissionBlocker, ValidationOutcomeView},
};
use serde_json::json;

#[test]
fn large_exponent_integer_data_is_compatible_without_canonical_expansion() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "quantity": { "type": "integer" }
        }
    }))
    .expect("the integer definition should compile");
    let quantity: serde_json::Value =
        serde_json::from_str("1e4096").expect("the integer should parse exactly");
    let form = definition
        .create_form(json!({ "quantity": quantity }))
        .expect("the integer form should be created");
    let quantity = form
        .node(form.view().root())
        .expect("the form root should exist")
        .children()
        .next()
        .expect("the integer control should exist");

    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    ));
    assert_eq!(
        form.node(quantity).unwrap().value_state(),
        Some(ScalarValueState::Compatible)
    );
}

#[test]
fn large_exponent_integer_constants_and_defaults_remain_representable() {
    let data_schema = serde_json::from_str(
        r#"{
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "fixed": {
                    "type": "integer",
                    "const": 1e4096
                },
                "values": {
                    "type": "array",
                    "items": {
                        "type": "integer",
                        "default": 1e4096
                    }
                }
            }
        }"#,
    )
    .expect("the data schema should parse exactly");
    let form_data = serde_json::from_str(r#"{"fixed":1e4096,"values":[]}"#)
        .expect("the form data should parse exactly");
    let definition = FormDefinition::compile(data_schema)
        .expect("the mathematical integer constant should compile");
    let mut form = definition
        .create_form(form_data)
        .expect("the integer form should be created");
    let values = form
        .node(form.view().root())
        .expect("the form root should exist")
        .children()
        .find(|identity| {
            form.node(*identity).is_some_and(|node| {
                node.binding()
                    .is_some_and(|binding| binding.pointer().as_str() == "/values")
            })
        })
        .expect("the integer array should exist");

    form.user()
        .append_item(values)
        .expect("the representable integer default should be appended");
    let expected: serde_json::Value = serde_json::from_str(r#"{"fixed":1e4096,"values":[1e4096]}"#)
        .expect("the expected data should parse exactly");
    assert_eq!(form.form_data(), &expected);
}

#[test]
fn arbitrary_precision_integer_edits_preserve_exact_state_through_submission() {
    let definition = FormDefinition::compiler(
        serde_json::from_str(
            r#"{
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": false,
                "required": ["quantity"],
                "properties": {
                    "quantity": {
                        "type": "integer",
                        "title": "Quantity",
                        "minimum": 184467440737095516160
                    }
                }
            }"#,
        )
        .expect("the trusted data schema should parse"),
    )
    .root_uri(
        RetrievalUri::parse("urn:schemaform:test:minimum")
            .expect("the root retrieval URI should be valid"),
    )
    .compile()
    .expect("the trusted data schema should compile");
    let baseline_data: serde_json::Value =
        serde_json::from_str(r#"{"quantity":184467440737095516160}"#)
            .expect("the arbitrary-precision baseline should parse");
    let mut form = definition
        .create_form(baseline_data.clone())
        .expect("the integer form should be created");
    let quantity = form
        .node(form.view().root())
        .expect("the form root should exist")
        .children()
        .next()
        .expect("the generated integer control should be instantiated");

    let before = form
        .node(quantity)
        .expect("the generated integer control should exist");
    assert_eq!(
        before.definition().semantic_kind(),
        Some(SemanticKind::Integer)
    );
    assert_eq!(
        before.display_text().as_deref(),
        Some("184467440737095516160")
    );
    assert_eq!(before.edit_buffer(), None);
    assert_eq!(before.parse_blocker(), None);
    assert!(!before.is_dirty());
    assert!(!before.is_touched());

    let before_data_revision = form.view().data_revision();
    let before_state_revision = form.view().state_revision();
    let blocked_transition = form
        .user()
        .input_text(quantity, "-")
        .expect("the incomplete integer should remain buffered");

    assert_eq!(
        blocked_transition.before_data_revision(),
        before_data_revision
    );
    assert_eq!(
        blocked_transition.after_data_revision(),
        before_data_revision
    );
    assert_eq!(
        blocked_transition.before_state_revision(),
        before_state_revision
    );
    assert_eq!(
        blocked_transition.after_state_revision(),
        form.view().state_revision()
    );
    assert_ne!(
        blocked_transition.before_state_revision(),
        blocked_transition.after_state_revision()
    );
    assert_eq!(blocked_transition.changed().collect::<Vec<_>>(), [quantity]);
    assert!(!blocked_transition.is_empty());
    assert_eq!(form.form_data(), &baseline_data);
    let blocked = form
        .node(quantity)
        .expect("the generated integer control should exist");
    assert_eq!(blocked.display_text().as_deref(), Some("-"));
    assert_eq!(blocked.edit_buffer(), Some("-"));
    assert_eq!(
        blocked.parse_blocker(),
        Some(ParseBlockerKind::InvalidInteger)
    );
    assert!(!blocked.is_dirty());
    assert!(!blocked.is_touched());
    let blocked_submission = form.prepare_submission();
    match blocked_submission.outcome() {
        SubmissionOutcome::Blocked(blockers) => assert!(blockers.iter().any(|blocker| {
            matches!(
                blocker,
                SubmissionBlocker::Parse {
                    target,
                    kind: ParseBlockerKind::InvalidInteger,
                } if *target == quantity
            )
        })),
        SubmissionOutcome::Ready(_) => panic!("the incomplete integer must block submission"),
    }

    form.user()
        .input_text(quantity, "184467440737095516159e0")
        .expect("the below-minimum integer should remain editable");

    let invalid_data: serde_json::Value =
        serde_json::from_str(r#"{"quantity":184467440737095516159}"#)
            .expect("the below-minimum form data should parse");
    assert_eq!(form.form_data(), &invalid_data);
    let invalid = form
        .node(quantity)
        .expect("the generated integer control should exist");
    assert_eq!(
        invalid.display_text().as_deref(),
        Some("184467440737095516159e0")
    );
    assert_eq!(invalid.parse_blocker(), None);
    let view = form.view();
    let findings = match view.validation_outcome() {
        ValidationOutcomeView::Invalid {
            findings,
            truncated: false,
        } => findings,
        _ => panic!("the below-minimum integer should be schema-invalid"),
    };
    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.code(), "minimum");
    assert_eq!(finding.instance_location().as_str(), "/quantity");
    assert_eq!(
        finding.keyword_location().resource().as_str(),
        "urn:schemaform:test:minimum"
    );
    assert_eq!(
        finding.keyword_location().pointer().as_str(),
        "/properties/quantity/minimum"
    );
    assert_eq!(
        finding.parameters(),
        &json!({ "limit": 184467440737095516160_u128 })
    );
    let validation_blocked = form.prepare_submission();
    match validation_blocked.outcome() {
        SubmissionOutcome::Blocked(blockers) => assert!(
            blockers
                .iter()
                .any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
        ),
        SubmissionOutcome::Ready(_) => panic!("schema-invalid form data must block submission"),
    }

    form.user()
        .input_text(quantity, "184467440737095516161e0")
        .expect("the corrected arbitrary-precision integer should parse");

    let corrected_data: serde_json::Value =
        serde_json::from_str(r#"{"quantity":184467440737095516161}"#)
            .expect("the corrected arbitrary-precision form data should parse");
    assert_eq!(form.form_data(), &corrected_data);
    let corrected = form
        .node(quantity)
        .expect("the generated integer control should exist");
    assert_eq!(
        corrected.display_text().as_deref(),
        Some("184467440737095516161e0")
    );
    assert_eq!(corrected.edit_buffer(), Some("184467440737095516161e0"));
    assert_eq!(corrected.parse_blocker(), None);
    assert!(corrected.is_dirty());
    assert!(!corrected.is_touched());

    form.user()
        .blur(quantity)
        .expect("blurring a valid integer should finalize its buffer");

    let blurred = form
        .node(quantity)
        .expect("the generated integer control should exist");
    assert_eq!(
        blurred.display_text().as_deref(),
        Some("184467440737095516161")
    );
    assert_eq!(blurred.edit_buffer(), None);
    assert_eq!(blurred.parse_blocker(), None);
    assert!(blurred.is_dirty());
    assert!(blurred.is_touched());

    let submitted = form.prepare_submission();
    let snapshot = match submitted.outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot.clone(),
        SubmissionOutcome::Blocked(_) => panic!("the corrected integer should be submittable"),
    };
    assert_eq!(snapshot.form_data(), &corrected_data);
    assert_eq!(
        serde_json::to_string(snapshot.form_data()).expect("the snapshot should serialize"),
        r#"{"quantity":184467440737095516161}"#
    );

    form.user()
        .input_text(quantity, "2")
        .expect("the form should remain editable after submission");
    assert_eq!(snapshot.form_data(), &corrected_data);
}

#[test]
fn definition_fingerprints_cover_validation_semantics() {
    let compile = |maximum| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "quantity": {
                    "type": "integer",
                    "maximum": maximum
                }
            }
        }))
        .expect("the trusted data schema should compile")
    };

    assert_ne!(compile(10).fingerprint(), compile(11).fingerprint());

    let annotated = |description, expected| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "description": description,
            "type": "object",
            "properties": {
                "quantity": {
                    "type": "integer",
                    "const": expected
                }
            }
        }))
        .expect("the trusted data schema should compile")
    };
    assert_eq!(
        annotated("first annotation", json!(1)).fingerprint(),
        annotated("second annotation", json!(1)).fingerprint()
    );
    assert_ne!(
        annotated("same annotation", json!(1)).fingerprint(),
        annotated("same annotation", json!(2)).fingerprint()
    );

    let enumerated = |options| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "quantity": {
                    "type": "integer",
                    "enum": options
                }
            }
        }))
        .expect("the trusted data schema should compile")
    };
    assert_eq!(
        enumerated(json!([1, 2])).fingerprint(),
        enumerated(json!([2, 1])).fingerprint()
    );
}

#[test]
fn validation_findings_are_retained_with_a_finite_bound() {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for index in 0..257 {
        let property = format!("field-{index:03}");
        properties.insert(property.clone(), json!({ "type": "string" }));
        required.push(serde_json::Value::String(property));
    }
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": properties,
        "required": required
    }))
    .expect("the trusted data schema should compile");
    let mut form = definition
        .create_form(json!({}))
        .expect("schema-invalid form data should remain constructible");

    match form.view().validation_outcome() {
        ValidationOutcomeView::Invalid {
            findings,
            truncated,
        } => {
            assert_eq!(findings.len(), 256);
            assert!(truncated);
            assert_eq!(
                findings[0].parameters(),
                &json!({ "property": "field-000" })
            );
            assert_eq!(
                findings[255].parameters(),
                &json!({ "property": "field-255" })
            );
        }
        _ => panic!("missing required properties should be schema-invalid"),
    }
    let preparation = form.prepare_submission();
    assert!(matches!(
        preparation.outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers
                .iter()
                .any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
                && blockers.iter().any(|blocker| matches!(
                    blocker,
                    SubmissionBlocker::ValidationFindingsTruncated { retained: 256 }
                ))
    ));
    assert!(form.view().visible_findings().any(|finding| matches!(
        finding,
        FindingView::ValidationFindingsTruncated { retained: 256, .. }
    )));
    assert!(
        form.node(form.view().root())
            .expect("the root should exist")
            .visible_findings()
            .any(|finding| matches!(
                finding,
                FindingView::ValidationFindingsTruncated { retained: 256, .. }
            ))
    );
}

#[test]
fn validator_rejects_nested_dialects_and_preserves_pointer_characters() {
    assert!(
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "quantity": {
                    "$schema": "http://json-schema.org/draft-07/schema#",
                    "type": "integer"
                }
            }
        }))
        .is_err()
    );
    assert!(
        FormDefinition::compiler(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "$schema": {
                    "type": "integer",
                    "minimum": 1,
                    "const": { "$schema": "instance data" }
                }
            }
        }))
        .analyze()
        .is_ok()
    );

    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "unit price": {
                "type": "integer",
                "minimum": 1
            }
        }
    }))
    .expect("the trusted data schema should compile");
    let form = definition
        .create_form(json!({ "unit price": 0 }))
        .expect("schema-invalid form data should remain constructible");
    match form.view().validation_outcome() {
        ValidationOutcomeView::Invalid { findings, .. } => assert_eq!(
            findings[0].keyword_location().pointer().as_str(),
            "/properties/unit price/minimum"
        ),
        _ => panic!("the below-minimum integer should be schema-invalid"),
    }
}
