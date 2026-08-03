use std::collections::BTreeSet;

use schemaform::{
    CompileError, ExternalFinding, ExternalFindingBatch, FindingView, Form, FormDefinition,
    InstanceIdentity, JsonPointer, RetrievalUri, SchemaResource, SubmissionOutcome,
    definition::{DefinitionNodeKind, SemanticKind},
    form::{
        ExternalFindingError, ParseBlockerKind, ScalarValueState, SubmissionBlocker,
        TransactionError, UserOperationError, ValidationOutcomeView,
    },
};
use serde_json::{Value, json};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn a_closed_empty_fixed_object_compiles_and_submits() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false
    }))
    .expect("an object with no possible members is a finite fixed object");
    let mut form = definition.create_form(json!({})).unwrap();

    assert!(
        form.node(form.view().root())
            .expect("the root should exist")
            .children()
            .next()
            .is_none()
    );
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(snapshot) if snapshot.form_data() == &json!({})
    ));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn every_fixed_object_scalar_and_presence_state_has_a_public_projection() {
    let definition = profile_definition();
    let cases = [
        (
            "/text",
            SemanticKind::String,
            json!(""),
            ScalarValueState::Empty,
        ),
        (
            "/text",
            SemanticKind::String,
            json!("present"),
            ScalarValueState::Compatible,
        ),
        (
            "/integer",
            SemanticKind::Integer,
            json!(1),
            ScalarValueState::Compatible,
        ),
        (
            "/number",
            SemanticKind::Number,
            exact_json("1.2345678901234567890123456789"),
            ScalarValueState::Compatible,
        ),
        (
            "/boolean",
            SemanticKind::Boolean,
            json!(true),
            ScalarValueState::Compatible,
        ),
        (
            "/choice",
            SemanticKind::Choice,
            json!(1),
            ScalarValueState::Compatible,
        ),
        (
            "/constant",
            SemanticKind::Choice,
            json!("EU"),
            ScalarValueState::Compatible,
        ),
        (
            "/nothing",
            SemanticKind::Null,
            Value::Null,
            ScalarValueState::Null,
        ),
    ];

    for (binding, kind, value, state) in cases {
        let property = binding.trim_start_matches('/');
        let form_data = object_with(property, value);
        let form = definition
            .create_form(form_data.clone())
            .expect("every supported scalar should instantiate");
        let node = form
            .node(node_with_binding(&form, binding))
            .expect("the supported scalar should have a public node projection");
        assert_eq!(node.definition().semantic_kind(), Some(kind));
        assert_eq!(node.value_state(), Some(state));
        assert_eq!(form.form_data(), &form_data);
        assert_eq!(node.edit_buffer(), None);
        assert_eq!(node.parse_blocker(), None);
        assert!(!node.is_touched());
        assert!(!node.is_dirty());
    }

    for (form_data, expected) in [
        (json!({}), ScalarValueState::Missing),
        (json!({ "nullable": null }), ScalarValueState::Null),
        (json!({ "nullable": "" }), ScalarValueState::Empty),
        (
            json!({ "nullable": "present" }),
            ScalarValueState::Compatible,
        ),
        (json!({ "nullable": 7 }), ScalarValueState::Incompatible),
    ] {
        let form = definition
            .create_form(form_data.clone())
            .expect("every scalar presence state should instantiate without coercion");
        let node = form
            .node(node_with_binding(&form, "/nullable"))
            .expect("the nullable control should remain represented");
        assert_eq!(node.value_state(), Some(expected));
        assert_eq!(form.form_data(), &form_data);
    }

    for (form_data, can_materialize, can_remove) in [
        (json!({}), true, false),
        (json!({ "optional_object": null }), false, true),
        (json!({ "optional_object": {} }), false, true),
        (
            json!({ "optional_object": { "label": "present" } }),
            false,
            true,
        ),
        (json!({ "optional_object": 7 }), false, true),
    ] {
        let form = definition
            .create_form(form_data.clone())
            .expect("every optional fixed-object presence state should remain constructible");
        let node = form
            .node(node_with_binding(&form, "/optional_object"))
            .expect("the optional fixed object should remain represented");
        assert_eq!(
            node.definition().semantic_kind(),
            Some(SemanticKind::FixedObject)
        );
        assert_eq!(node.allowed_operations().can_materialize(), can_materialize);
        assert_eq!(node.allowed_operations().can_remove_value(), can_remove);
        assert_eq!(form.form_data(), &form_data);
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn fixed_object_authority_and_lifecycle_trace_has_stable_public_outcomes() {
    let initial = json!({
        "text": "Ada",
        "integer": 2,
        "number": 1.25,
        "boolean": true,
        "choice": "standard",
        "constant": "EU",
        "nothing": null,
        "nullable": "present",
        "nested": { "label": "Nested" },
        "read_only": "host owned",
        "secret": "existing secret",
        "undeclared": { "preserved": true }
    });
    let mut form = profile_definition()
        .create_form(initial.clone())
        .expect("the complete fixed-object trace should instantiate");
    let text = node_with_binding(&form, "/text");
    let integer = node_with_binding(&form, "/integer");
    let boolean = node_with_binding(&form, "/boolean");
    let choice = node_with_binding(&form, "/choice");
    let nullable = node_with_binding(&form, "/nullable");
    let optional_object = node_with_binding(&form, "/optional_object");
    let optional_label = node_with_binding(&form, "/optional_object/label");
    let read_only = node_with_binding(&form, "/read_only");
    let secret = node_with_binding(&form, "/secret");

    let parse_transition = form
        .user()
        .input_text(integer, "-")
        .expect("incomplete integer text should remain buffered");
    assert_revision_delta(&parse_transition, false, true);
    assert_changed_bindings(&form, &parse_transition, ["/integer"]);
    assert_eq!(form.form_data(), &initial);
    let integer_node = form
        .node(integer)
        .expect("the integer should remain represented");
    assert_eq!(integer_node.edit_buffer(), Some("-"));
    assert_eq!(
        integer_node.parse_blocker(),
        Some(ParseBlockerKind::InvalidInteger)
    );
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Parse { target, kind: ParseBlockerKind::InvalidInteger }
                    if *target == integer
            ))
    ));

    let exact_integer = exact_json("1234567890123456789012345678901234567890");
    let integer_transition = form
        .user()
        .input_text(integer, exact_integer.to_string())
        .expect("an arbitrary-precision integer should become canonical form data");
    assert_revision_delta(&integer_transition, true, true);
    assert_changed_contains(&form, &integer_transition, "/integer");
    assert_eq!(form.form_data()["integer"], exact_integer);
    let blur_transition = form
        .user()
        .blur(integer)
        .expect("blur should finalize the parseable edit buffer");
    assert_revision_delta(&blur_transition, false, true);
    assert_changed_contains(&form, &blur_transition, "/integer");
    let integer_node = form
        .node(integer)
        .expect("the integer should remain represented");
    assert_eq!(integer_node.edit_buffer(), None);
    assert!(integer_node.is_touched());
    assert!(integer_node.is_dirty());

    let set_null = form
        .user()
        .set_null(nullable)
        .expect("the nullable scalar should accept null");
    assert_revision_delta(&set_null, true, true);
    assert_changed_contains(&form, &set_null, "/nullable");
    let removed_nullable = form
        .user()
        .remove_value(nullable)
        .expect("the optional scalar should be removable");
    assert_revision_delta(&removed_nullable, true, true);
    assert_changed_contains(&form, &removed_nullable, "/nullable");
    let restored_nullable = form
        .user()
        .input_text(nullable, "restored")
        .expect("text input should explicitly restore a missing scalar");
    assert_changed_contains(&form, &restored_nullable, "/nullable");
    let boolean_transition = form
        .user()
        .set_value(boolean, json!(false))
        .expect("the boolean should accept its typed operation");
    assert_changed_contains(&form, &boolean_transition, "/boolean");
    let choice_transition = form
        .user()
        .set_value(choice, json!(1))
        .expect("the finite choice should retain its exact JSON kind");
    assert_changed_contains(&form, &choice_transition, "/choice");

    let materialized = form
        .user()
        .materialize(optional_object)
        .expect("the optional fixed object should materialize explicitly");
    assert_revision_delta(&materialized, true, true);
    assert_changed_contains(&form, &materialized, "/optional_object");
    assert_eq!(form.form_data()["optional_object"], json!({}));
    let child_transition = form
        .user()
        .input_text(optional_label, "created")
        .expect("a child of the materialized object should become editable");
    assert_changed_contains(&form, &child_transition, "/optional_object/label");
    let removed_object = form
        .user()
        .remove_value(optional_object)
        .expect("the optional fixed object should be removable explicitly");
    assert_changed_contains(&form, &removed_object, "/optional_object");
    assert!(form.form_data().get("optional_object").is_none());

    let rejected_revisions = (form.view().data_revision(), form.view().state_revision());
    assert_eq!(
        form.user().input_text(read_only, "user write"),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        rejected_revisions
    );
    let secret_transition = form
        .user()
        .replace_value(secret, json!("replacement secret"))
        .expect("write-only data should permit explicit replacement");
    assert_changed_contains(&form, &secret_transition, "/secret");

    let external_revision = form.view().data_revision();
    let external_transition = form
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            external_revision,
            [ExternalFinding::blocking(
                "name-taken",
                pointer("/text"),
                json!({ "attempt": 1 }),
            )],
        ))
        .expect("a current external finding should apply");
    assert_revision_delta(&external_transition, false, true);
    assert_changed_bindings(&form, &external_transition, ["/text"]);
    let replacement_transition = form
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            external_revision,
            [ExternalFinding::blocking(
                "name-still-taken",
                pointer("/text"),
                json!({ "attempt": 2 }),
            )],
        ))
        .expect("a source should replace its current external batch");
    assert_revision_delta(&replacement_transition, false, true);
    assert_changed_bindings(&form, &replacement_transition, ["/text"]);
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::External { source, finding }
                    if source == "server" && finding.code() == "name-still-taken"
            ))
    ));

    let before_rollback = form.form_data().clone();
    let rollback_revisions = (form.view().data_revision(), form.view().state_revision());
    let rollback = form.try_transact(|draft| {
        draft.set(&pointer("/read_only"), json!("not committed"));
        Err::<(), _>("abort")
    });
    assert!(matches!(rollback, Err(TransactionError::Closure("abort"))));
    assert_eq!(form.form_data(), &before_rollback);
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        rollback_revisions
    );

    let host_transition = form
        .transact(|draft| {
            draft.set(&pointer("/read_only"), json!("host replacement"));
            draft.set(&pointer("/undeclared/host"), json!(true));
        })
        .expect("one privileged host transaction should publish atomically");
    assert_revision_delta(&host_transition, true, true);
    assert_changed_contains(&form, &host_transition, "/read_only");
    assert_eq!(form.form_data()["read_only"], json!("host replacement"));
    assert_eq!(form.form_data()["undeclared"]["host"], json!(true));
    assert!(
        form.view()
            .visible_findings()
            .all(|finding| !matches!(finding, FindingView::External { .. }))
    );
    let before_stale = (form.view().data_revision(), form.view().state_revision());
    assert!(matches!(
        form.apply_external_findings(ExternalFindingBatch::new(
            "server",
            external_revision,
            [ExternalFinding::blocking(
                "stale",
                pointer("/text"),
                json!({}),
            )],
        )),
        Err(ExternalFindingError::StaleRevision { .. })
    ));
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        before_stale
    );

    let text_transition = form
        .user()
        .input_text(text, "Grace")
        .expect("the ordinary string control should remain editable");
    assert_changed_contains(&form, &text_transition, "/text");
    let snapshot = match form.prepare_submission().outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot.clone(),
        SubmissionOutcome::Blocked(_) => panic!("the repaired fixed-object trace should submit"),
    };
    assert_eq!(snapshot.form_data(), form.form_data());
    assert_eq!(snapshot.data_revision(), form.view().data_revision());
    assert_eq!(
        snapshot.definition_fingerprint(),
        form.definition().fingerprint()
    );

    let reset = form.reset();
    assert_revision_delta(&reset, true, true);
    assert_changed_contains(&form, &reset, "/nullable");
    assert_eq!(form.form_data(), &initial);
    assert!(!form.view().submission_attempted());
    for identity in [text, integer, nullable, read_only, secret] {
        let node = form
            .node(identity)
            .expect("fixed-object identities should survive reset");
        assert_eq!(node.edit_buffer(), None);
        assert_eq!(node.parse_blocker(), None);
        assert!(!node.is_touched());
        assert!(!node.is_dirty());
    }

    let reinitialized = form
        .reinitialize(initial.clone())
        .expect("semantically equal data should still start a fresh lifecycle");
    assert_revision_delta(&reinitialized, true, true);
    assert!(
        reinitialized.changed().next().is_none(),
        "equal clean fixed-object reinitialization changes only form-level revisions"
    );
    assert_eq!(form.form_data(), &initial);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn fixed_object_finding_families_and_submission_outcomes_are_exhaustive() {
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["age", "contact", "name", "quantity"],
        "properties": {
            "age": { "type": "integer", "minimum": 18 },
            "contact": { "oneOf": [{ "type": "string" }, { "type": "integer" }] },
            "name": { "type": "string", "minLength": 3 },
            "quantity": { "type": "integer" }
        }
    }))
    .analyze()
    .expect("the deferred region should remain finite in lenient mode")
    .into_parts()
    .0;
    let mut form = definition
        .create_form(json!({ "age": 1, "contact": "Ada", "name": "", "quantity": 1 }))
        .expect("invalid and unsupported fixed-object data should remain constructible");
    let quantity = node_with_binding(&form, "/quantity");
    form.user()
        .input_text(quantity, "-")
        .expect("the parse-blocked spelling should be retained");
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "server-rejected",
            pointer("/missing"),
            json!({ "retry": false }),
        )],
    ))
    .expect("the external finding should apply at the current revision");

    let preparation = form.prepare_submission();
    assert_revision_delta(preparation.transition(), false, true);
    let blockers = match preparation.outcome() {
        SubmissionOutcome::Blocked(blockers) => blockers,
        SubmissionOutcome::Ready(_) => panic!("all current blocker families must be returned"),
    };
    let blocker_kinds = blockers
        .iter()
        .map(|blocker| match blocker {
            SubmissionBlocker::Parse { .. } => "parse",
            SubmissionBlocker::Validation(_) => "validation",
            SubmissionBlocker::ValidationFindingsTruncated { .. } => "validation-truncated",
            SubmissionBlocker::Indeterminate(_) => "indeterminate",
            SubmissionBlocker::Capability(_) => "capability",
            SubmissionBlocker::External { .. } => "external",
            _ => "unknown",
        })
        .collect::<Vec<_>>();
    assert_eq!(
        blocker_kinds,
        [
            "parse",
            "validation",
            "validation",
            "capability",
            "external"
        ]
    );
    assert!(form.view().submission_attempted());
    let visible_families = form
        .view()
        .visible_findings()
        .map(|finding| match finding {
            FindingView::Validation { .. } => "validation",
            FindingView::ValidationFindingsTruncated { .. } => "validation-truncated",
            FindingView::Indeterminate { .. } => "indeterminate",
            FindingView::Capability { .. } => "capability",
            FindingView::External { .. } => "external",
            FindingView::Parse { .. } => "parse",
            _ => "unknown",
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        visible_families,
        BTreeSet::from(["capability", "external", "parse", "validation"])
    );

    let (truncated_definition, empty_data) = validation_truncation_fixture();
    let mut truncated = truncated_definition
        .create_form(empty_data)
        .expect("the many-finding object should remain constructible");
    assert!(matches!(
        truncated.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, truncated: true } if findings.len() == 256
    ));
    assert!(matches!(
        truncated.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().filter(|blocker| matches!(blocker, SubmissionBlocker::Validation(_))).count() == 256
                && blockers.iter().any(|blocker| matches!(
                    blocker,
                    SubmissionBlocker::ValidationFindingsTruncated { retained: 256 }
                ))
    ));
}

#[cfg(schemaform_test_validation_faults)]
#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn indeterminate_validation_discards_partial_findings_and_blocks_submission() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "x-schemaform-test-validation-fault": 2,
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "role"],
        "properties": {
            "name": { "type": "string", "minLength": 3 },
            "role": { "type": "string", "minLength": 3 }
        }
    }))
    .expect("the private validator fault fixture should compile");
    let mut form = definition
        .create_form(json!({ "name": "", "role": "editor" }))
        .expect("the initially invalid form should remain constructible");

    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, truncated: false }
            if findings.len() == 1 && findings[0].code() == "minLength"
    ));
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "name-rejected",
            pointer("/name"),
            json!({}),
        )],
    ))
    .expect("a current external finding should apply before the faulted commit");

    let transition = form
        .transact(|draft| draft.set(&pointer("/role"), json!("")))
        .expect("structurally permitted data should commit despite evaluator failure");
    assert_revision_delta(&transition, true, true);
    assert_eq!(form.form_data(), &json!({ "name": "", "role": "" }));
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Indeterminate(reason)
            if reason.code() == "injected-validator-failure"
    ));
    assert!(form.view().visible_findings().all(|finding| !matches!(
        finding,
        FindingView::Validation { .. }
            | FindingView::ValidationFindingsTruncated { .. }
            | FindingView::External { .. }
    )));

    let preparation = form.prepare_submission();
    assert_revision_delta(preparation.transition(), false, true);
    assert!(matches!(
        preparation.outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().all(|blocker| matches!(blocker, SubmissionBlocker::Indeterminate(_)))
                && blockers.iter().count() == 1
    ));
    assert!(form.view().visible_findings().any(|finding| matches!(
        finding,
        FindingView::Indeterminate { reason, .. }
            if reason.code() == "injected-validator-failure"
    )));
    assert!(form.view().visible_findings().all(|finding| !matches!(
        finding,
        FindingView::Validation { .. } | FindingView::ValidationFindingsTruncated { .. }
    )));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn fixed_projection_map_constraints_warn_and_preserve_undeclared_data() {
    let mut closed_pattern_schema =
        fixed_projection_schema("patternProperties", json!({ "^x-": { "type": "integer" } }));
    closed_pattern_schema
        .as_object_mut()
        .expect("the fixture schema should be an object")
        .insert("additionalProperties".to_owned(), Value::Bool(false));
    for (schema, expected_codes) in [
        (
            fixed_projection_schema("additionalProperties", json!({ "type": "integer" })),
            &["applicator.additional-properties.schema-projection"][..],
        ),
        (
            fixed_projection_schema("patternProperties", json!({ "^x-": { "type": "integer" } })),
            &[
                "applicator.pattern-properties.fixed-projection",
                "applicator.additional-properties.open",
            ][..],
        ),
        (
            closed_pattern_schema,
            &["applicator.pattern-properties.fixed-projection"][..],
        ),
    ] {
        let definition = FormDefinition::compiler(schema.clone())
            .compile()
            .expect("a fixed declared projection should compile with a map warning");
        let analysis = FormDefinition::compiler(schema)
            .analyze()
            .expect("the warning should have the same lenient outcome");

        let strict_findings = definition.capability_findings().collect::<Vec<_>>();
        let lenient_findings = analysis.capability_report().findings().collect::<Vec<_>>();
        for findings in [strict_findings, lenient_findings] {
            assert_eq!(
                findings
                    .iter()
                    .map(|finding| finding.code())
                    .collect::<Vec<_>>(),
                expected_codes
            );
            assert!(
                findings
                    .iter()
                    .all(|finding| finding.instance_location().as_str().is_empty())
            );
            assert!(findings.iter().all(|finding| !finding.is_blocking()));
        }

        let form_data = json!({ "name": "Ada", "x-extra": 7 });
        let mut form = definition
            .create_form(form_data.clone())
            .expect("undeclared form data should remain constructible");
        assert_eq!(form.form_data(), &form_data);
        assert!(matches!(
            form.prepare_submission().outcome(),
            SubmissionOutcome::Ready(snapshot) if snapshot.form_data() == &form_data
        ));

        form.transact(|draft| draft.set(&pointer("/x-extra"), json!("invalid")))
            .expect("the host should be able to install invalid undeclared data");
        assert_eq!(form.form_data()["x-extra"], json!("invalid"));
        assert!(matches!(
            form.prepare_submission().outcome(),
            SubmissionOutcome::Blocked(blockers)
                if blockers.iter().any(|blocker| matches!(
                    blocker,
                    SubmissionBlocker::Validation(finding)
                        if finding.instance_location().as_str() == "/x-extra"
                ))
        ));
        assert_eq!(form.form_data()["x-extra"], json!("invalid"));
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn deferred_fixed_object_shapes_have_matching_strict_and_lenient_outcomes() {
    let cases = [
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "target": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "child": { "$ref": "#/$defs/node" }
                        }
                    }
                },
                "$defs": {
                    "node": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "child": { "$ref": "#/$defs/node" }
                        }
                    }
                }
            }),
            json!({ "target": { "child": { "child": {} } } }),
            &["structure.recursive.projection"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$dynamicAnchor": "node",
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "target": { "$dynamicRef": "#node" }
                }
            }),
            json!({ "target": {} }),
            &["core.dynamic-reference.shape"][..],
        ),
        (
            deferred_property_schema(json!({
                "oneOf": [{ "type": "string" }, { "type": "integer" }]
            })),
            json!({ "target": "preserved" }),
            &["applicator.one-of"][..],
        ),
        (
            deferred_property_schema(json!({
                "type": "string",
                "anyOf": [{ "minLength": 2 }, { "pattern": "^[A-Z]" }]
            })),
            json!({ "target": "preserved" }),
            &["applicator.any-of"][..],
        ),
        (
            deferred_property_schema(json!({
                "type": "array",
                "prefixItems": [{ "type": "string" }, { "type": "integer" }]
            })),
            json!({ "target": ["preserved", 1] }),
            &["applicator.prefix-items"][..],
        ),
        (
            deferred_property_schema(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "kind": { "type": "string" } },
                "if": { "properties": { "kind": { "const": "business" } } },
                "then": { "properties": { "taxId": { "type": "string" } } },
                "else": { "properties": { "nickname": { "type": "string" } } }
            })),
            json!({ "target": { "kind": "business", "taxId": "preserved" } }),
            &[
                "applicator.else.structural",
                "applicator.properties.conditional",
                "applicator.if.structural",
                "applicator.then.structural",
                "applicator.properties.conditional",
            ][..],
        ),
        (
            deferred_property_schema(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "kind": { "type": "string" } },
                "allOf": [{
                    "if": { "properties": { "kind": { "const": "business" } } },
                    "then": { "properties": { "detail": { "type": "string" } } }
                }]
            })),
            json!({ "target": { "kind": "business", "detail": "preserved" } }),
            &[
                "applicator.all-of.conditional",
                "applicator.if.structural",
                "applicator.then.structural",
                "applicator.properties.conditional",
            ][..],
        ),
        (
            deferred_property_schema(json!({
                "type": "string",
                "if": { "minLength": 1 },
                "then": { "enum": ["one", "two"] }
            })),
            json!({ "target": "preserved" }),
            &[
                "applicator.if.structural",
                "applicator.then.structural",
                "validation.enum.conditional",
            ][..],
        ),
        (
            deferred_property_schema(json!({
                "allOf": [{ "type": "string" }, { "type": "integer" }]
            })),
            json!({ "target": "preserved" }),
            &["applicator.all-of.ambiguous"][..],
        ),
        (
            deferred_property_schema(json!({
                "enum": ["scalar", { "preserved": true }]
            })),
            json!({ "target": { "preserved": true } }),
            &["validation.enum.structured"][..],
        ),
        (
            deferred_property_schema(json!({
                "const": { "preserved": true }
            })),
            json!({ "target": { "preserved": true } }),
            &["validation.const.structured"][..],
        ),
    ];

    for (schema, form_data, expected_codes) in cases {
        let strict = strict_report(schema.clone());
        let analysis = FormDefinition::compiler(schema)
            .analyze()
            .expect("every deferred shape should have a finite lenient definition");
        assert_eq!(&strict, analysis.capability_report());
        assert_eq!(
            strict
                .findings()
                .map(|finding| finding.code())
                .collect::<Vec<_>>(),
            expected_codes
        );
        assert!(strict.findings().all(|finding| finding.is_blocking()));
        assert!(has_unsupported_node(analysis.definition()));

        let definition = analysis.into_parts().0;
        let mut form = definition
            .create_form(form_data.clone())
            .expect("lenient fixed-object data should remain constructible");
        assert_eq!(form.form_data(), &form_data);
        assert!(matches!(
            form.prepare_submission().outcome(),
            SubmissionOutcome::Blocked(blockers)
                if blockers.iter().any(|blocker| matches!(
                    blocker,
                    SubmissionBlocker::Capability(_)
                ))
        ));
        assert_eq!(form.form_data(), &form_data);
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn every_applicable_deferred_fixed_object_construct_has_a_typed_outcome() {
    let cases = [
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "string"
            }),
            &["structure.root.scalar"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "enum": ["one", "two"]
            }),
            &["structure.root.scalar"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "const": 7
            }),
            &["structure.root.scalar"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "enum": [{ "structured": true }]
            }),
            &["validation.enum.structured"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "const": ["structured"]
            }),
            &["validation.const.structured"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "array",
                "items": { "type": "string" }
            }),
            &["structure.root.array"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": ["array", "string"]
            }),
            &["validation.type.ambiguous"][..],
        ),
        (
            deferred_property_schema(json!({ "type": ["string", "integer"] })),
            &["validation.type.ambiguous"][..],
        ),
        (
            deferred_property_schema(json!({})),
            &["validation.type.unconstrained"][..],
        ),
        (
            deferred_property_schema(Value::Bool(true)),
            &["core.boolean.unconstrained"][..],
        ),
        (
            deferred_property_schema(json!({ "not": { "type": "string" } })),
            &["applicator.not.shape"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": { "type": "string" }
            }),
            &["applicator.additional-properties.dynamic-map"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": true
            }),
            &["applicator.additional-properties.dynamic-map"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object"
            }),
            &["applicator.additional-properties.dynamic-map"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "patternProperties": { "^x-": { "type": "string" } },
                "additionalProperties": false
            }),
            &["applicator.pattern-properties.shape"][..],
        ),
        (
            deferred_property_schema(json!({
                "type": "object",
                "additionalProperties": false,
                "properties": { "kind": { "type": "string" } },
                "dependentSchemas": {
                    "kind": { "properties": { "detail": { "type": "string" } } }
                }
            })),
            &["applicator.dependent-schemas.structural"][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "unevaluatedProperties": { "type": "string" }
            }),
            &[
                "applicator.additional-properties.dynamic-map",
                "unevaluated.properties.shape",
            ][..],
        ),
        (
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": { "type": "string" },
                "patternProperties": { "^x-": { "type": "string" } },
                "unevaluatedProperties": false
            }),
            &[
                "applicator.additional-properties.dynamic-map",
                "applicator.pattern-properties.shape",
                "unevaluated.properties.shape",
            ][..],
        ),
    ];

    for (schema, expected_codes) in cases {
        let strict = strict_report(schema.clone());
        let analysis = FormDefinition::compiler(schema)
            .analyze()
            .expect("a deferred construct should produce a finite lenient definition");
        assert_eq!(&strict, analysis.capability_report());
        assert_eq!(
            strict
                .findings()
                .map(|finding| finding.code())
                .collect::<Vec<_>>(),
            expected_codes
        );
        assert!(strict.findings().all(|finding| finding.is_blocking()));
        assert!(has_unsupported_node(analysis.definition()));

        let mut form = analysis
            .into_parts()
            .0
            .create_form(json!({ "target": { "preserved": true } }))
            .expect("lenient form data should remain preserved");
        let preserved = form.form_data().clone();
        assert!(matches!(
            form.prepare_submission().outcome(),
            SubmissionOutcome::Blocked(blockers)
                if blockers.iter().any(|blocker| matches!(
                    blocker,
                    SubmissionBlocker::Capability(finding)
                        if expected_codes.contains(&finding.code())
                ))
        ));
        assert_eq!(form.form_data(), &preserved);
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn referenced_type_outcomes_keep_their_resource_and_pointer_together() {
    let resource_uri = RetrievalUri::parse("https://schemas.example/ambiguous.json")
        .expect("the fixture resource URI should be valid");
    let compiler = || {
        FormDefinition::compiler(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "target": { "$ref": "https://schemas.example/ambiguous.json" }
            }
        }))
        .resource(SchemaResource::new(
            resource_uri.clone(),
            json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": ["string", "integer"]
            }),
        ))
    };
    let strict = match compiler().compile() {
        Err(CompileError::Capability(report)) => report,
        Err(error) => panic!("strict compilation returned the wrong error: {error}"),
        Ok(_) => panic!("the ambiguous referenced type should be blocked"),
    };
    let analysis = compiler()
        .analyze()
        .expect("the ambiguous referenced type should remain analyzable");
    assert_eq!(&strict, analysis.capability_report());
    let finding = strict
        .findings()
        .find(|finding| finding.code() == "validation.type.ambiguous")
        .expect("the referenced type finding should exist");
    assert_eq!(
        finding.keyword_location().resource().as_str(),
        resource_uri.as_str()
    );
    assert_eq!(finding.keyword_location().pointer().as_str(), "/type");
}

fn profile_definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": [
            "text",
            "integer",
            "number",
            "boolean",
            "choice",
            "constant",
            "nothing",
            "nested",
            "read_only",
            "secret"
        ],
        "properties": {
            "text": { "type": "string", "minLength": 1 },
            "integer": { "type": "integer", "minimum": 1 },
            "number": { "type": "number" },
            "boolean": { "type": "boolean" },
            "choice": { "enum": ["standard", 1, true, null] },
            "constant": { "const": "EU" },
            "nothing": { "type": "null" },
            "nullable": { "type": ["string", "null"] },
            "nested": {
                "type": "object",
                "additionalProperties": false,
                "required": ["label"],
                "properties": {
                    "label": { "type": "string" }
                }
            },
            "optional_object": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "label": { "type": "string" }
                }
            },
            "read_only": { "type": "string", "readOnly": true },
            "secret": { "type": "string", "writeOnly": true }
        }
    }))
    .expect("the complete fixed-object profile schema should compile")
}

fn validation_truncation_fixture() -> (FormDefinition, Value) {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for index in 0..300 {
        let property = format!("required_{index:03}");
        properties.insert(property.clone(), json!({ "type": "string" }));
        required.push(Value::String(property));
    }
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties
    }))
    .expect("the validation truncation fixture should compile");
    (definition, json!({}))
}

fn deferred_property_schema(property: Value) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "target": property
        }
    })
}

fn fixed_projection_schema(keyword: &str, constraint: Value) -> Value {
    let mut schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        }
    });
    schema
        .as_object_mut()
        .expect("the fixture schema should be an object")
        .insert(keyword.to_owned(), constraint);
    schema
}

fn strict_report(schema: Value) -> schemaform::CapabilityReport {
    match FormDefinition::compiler(schema).compile() {
        Err(CompileError::Capability(report)) => report,
        Err(error) => panic!("strict compilation returned the wrong error: {error}"),
        Ok(_) => panic!("strict compilation should reject this fixture"),
    }
}

fn exact_json(value: &str) -> Value {
    serde_json::from_str(value).expect("the arbitrary-precision fixture should parse")
}

fn object_with(property: &str, value: Value) -> Value {
    Value::Object(serde_json::Map::from_iter([(property.to_owned(), value)]))
}

fn has_unsupported_node(definition: &FormDefinition) -> bool {
    let mut pending = vec![definition.root()];
    while let Some(identity) = pending.pop() {
        let node = definition
            .node(identity)
            .expect("the definition node should exist");
        if node.kind() == DefinitionNodeKind::Unsupported {
            return true;
        }
        pending.extend(node.children());
    }
    false
}

fn pointer(value: &str) -> JsonPointer {
    JsonPointer::parse(value).expect("the fixture pointer should be valid")
}

fn node_with_binding(form: &Form, binding: &str) -> InstanceIdentity {
    let mut pending = vec![form.view().root()];
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).expect("the form node should exist");
        if node
            .binding()
            .is_some_and(|current| current.pointer().as_str() == binding)
        {
            return identity;
        }
        let mut children = node.children().collect::<Vec<_>>();
        children.reverse();
        pending.extend(children);
    }
    panic!("the bound node should exist: {binding}")
}

fn assert_revision_delta(
    transition: &schemaform::Transition,
    data_changed: bool,
    state_changed: bool,
) {
    assert_eq!(
        transition.before_data_revision() != transition.after_data_revision(),
        data_changed
    );
    assert_eq!(
        transition.before_state_revision() != transition.after_state_revision(),
        state_changed
    );
    assert!(
        transition.removed().next().is_none(),
        "fixed-object transitions must not remove definition-owned identities"
    );
}

fn assert_changed_contains(form: &Form, transition: &schemaform::Transition, expected: &str) {
    let changed = transition
        .changed()
        .filter_map(|identity| form.node(identity))
        .filter_map(|node| {
            node.binding()
                .map(|binding| binding.pointer().as_str().to_owned())
        })
        .collect::<BTreeSet<_>>();
    assert!(
        changed.contains(expected),
        "the transition should include {expected}; changed bindings: {changed:?}"
    );
}

fn assert_changed_bindings<const N: usize>(
    form: &Form,
    transition: &schemaform::Transition,
    expected: [&str; N],
) {
    let actual = transition
        .changed()
        .map(|identity| {
            form.node(identity)
                .and_then(|node| {
                    node.binding()
                        .map(|binding| binding.pointer().as_str().to_owned())
                })
                .unwrap_or_default()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected.into_iter().map(str::to_owned).collect());
}
