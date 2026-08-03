use schemaform::{
    CompileError, FormDefinition, InstanceIdentity, JsonPointer, SubmissionOutcome,
    definition::SemanticKind,
    form::{UserOperationError, ValidationOutcomeView},
};
use serde_json::{Value, json};

#[test]
fn mixed_scalar_choices_round_trip_exact_values_through_form_state() {
    let definition = FormDefinition::compile(
        serde_json::from_str(
            r#"{
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "required": ["choice"],
                "properties": {
                    "choice": {
                        "title": "Choice",
                        "enum": [null, true, "true", 1.0000000000000000000000000000000000000001, "1.0000000000000000000000000000000000000001"]
                    }
                }
            }"#,
        )
        .expect("the arbitrary-precision choice schema should parse"),
    )
    .expect("finite scalar choices should compile");
    let baseline: Value =
        serde_json::from_str(r#"{"choice":1.0000000000000000000000000000000000000001}"#)
            .expect("the arbitrary-precision baseline should parse");
    let mut form = definition
        .create_form(baseline.clone())
        .expect("the choice form should be created");
    let choice = control_with_binding(&form, "/choice");
    let node = form.node(choice).expect("the choice control should exist");

    assert_eq!(
        node.definition().semantic_kind(),
        Some(SemanticKind::Choice)
    );
    let options = node
        .definition()
        .choice_options()
        .map(|option| (option.value().clone(), option.label().to_owned()))
        .collect::<Vec<_>>();
    assert_eq!(options.len(), 5);
    assert!(options.contains(&(Value::Null, "null".to_owned())));
    assert!(options.contains(&(Value::Bool(true), "true".to_owned())));
    assert!(options.contains(&(Value::String("true".to_owned()), "true".to_owned())));
    assert!(options.contains(&(
        Value::String("1.0000000000000000000000000000000000000001".to_owned()),
        "1.0000000000000000000000000000000000000001".to_owned(),
    )));
    assert_eq!(
        node.selected_choice().map(|option| option.value()),
        node.current_data()
    );
    assert!(node.allowed_operations().can_set_value());
    assert!(!node.is_dirty());

    let before = (form.view().data_revision(), form.view().state_revision());
    let transition = form
        .user()
        .set_value(choice, json!("true"))
        .expect("the string option should be selectable");
    assert_eq!(transition.changed().collect::<Vec<_>>(), [choice]);
    assert_eq!(form.form_data(), &json!({ "choice": "true" }));
    assert!(
        form.node(choice)
            .expect("the choice should remain")
            .is_dirty()
    );
    assert_ne!(
        (form.view().data_revision(), form.view().state_revision()),
        before
    );

    form.user()
        .set_value(choice, Value::Bool(true))
        .expect("the boolean option with the same label should remain distinct");
    assert_eq!(form.form_data(), &json!({ "choice": true }));
    assert_eq!(
        form.user().set_value(choice, json!(false)),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(form.form_data(), &json!({ "choice": true }));

    let choice_pointer = JsonPointer::parse("/choice").expect("the choice pointer should be valid");
    form.transact(|draft| draft.set(&choice_pointer, json!(false)))
        .expect("the host should be able to install an invalid choice");
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "enum")
    ));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));
    form.user()
        .set_null(choice)
        .expect("a compiled option should repair invalid form data");
    assert_eq!(form.form_data(), &json!({ "choice": null }));
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );

    form.user()
        .set_value(choice, baseline["choice"].clone())
        .expect("the exact number option should be selectable");
    assert!(
        !form
            .node(choice)
            .expect("the choice should remain")
            .is_dirty()
    );
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
    let snapshot = match form.prepare_submission().outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot.clone(),
        SubmissionOutcome::Blocked(_) => panic!("the selected option should be submittable"),
    };
    assert_eq!(snapshot.form_data(), &baseline);
}

#[test]
fn applicable_choice_constraints_intersect_and_compare_numbers_mathematically() {
    let schema = |first, second| {
        serde_json::from_str::<Value>(&format!(
            r#"{{
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {{
                    "value": {{
                        "allOf": [
                            {{"enum": {first}}},
                            {{"enum": {second}}},
                            {{"type": ["number", "null"]}}
                        ]
                    }}
                }}
            }}"#
        ))
        .expect("the applicable choice schema should parse")
    };
    let definition = FormDefinition::compile(schema(r#"[null, 1.0, "excluded"]"#, "[1e0, null]"))
        .expect("compatible applicable choices should compile");
    let reordered = FormDefinition::compile(schema(r#"["excluded", 1.0, null]"#, "[null, 1e0]"))
        .expect("choice ordering should not affect compilation");
    assert_eq!(definition.fingerprint(), reordered.fingerprint());

    let initial = serde_json::from_str(r#"{"value":1e0}"#)
        .expect("the alternate numeric representation should parse");
    let mut form = definition
        .create_form(initial)
        .expect("the applicable choice form should be created");
    let value = control_with_binding(&form, "/value");
    let numeric_option = form
        .node(value)
        .expect("the choice should exist")
        .definition()
        .choice_options()
        .find(|option| option.value().is_number())
        .expect("the numeric intersection should remain")
        .value()
        .clone();
    assert_eq!(
        form.node(value)
            .expect("the choice should exist")
            .definition()
            .choice_options()
            .count(),
        2
    );
    let before = (form.view().data_revision(), form.view().state_revision());
    let transition = form
        .user()
        .set_value(value, numeric_option)
        .expect("the mathematically equal option should be accepted");
    assert!(transition.is_empty());
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        before
    );
    assert_eq!(
        serde_json::to_string(form.form_data()).expect("form data should serialize"),
        r#"{"value":1e+0}"#
    );
}

#[test]
fn null_only_and_scalar_constants_are_labeled_fixed_choices() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["nothing", "region"],
        "properties": {
            "nothing": { "type": ["null"], "title": "Nothing" },
            "region": { "const": "EU", "title": "Region" }
        }
    }))
    .expect("null-only and scalar constants should compile");
    let form = definition
        .create_form(json!({ "nothing": null, "region": "EU" }))
        .expect("the fixed choice form should be created");

    for (binding, kind, value, label) in [
        ("/nothing", SemanticKind::Null, Value::Null, "null"),
        (
            "/region",
            SemanticKind::Choice,
            Value::String("EU".to_owned()),
            "EU",
        ),
    ] {
        let identity = control_with_binding(&form, binding);
        let node = form.node(identity).expect("the fixed control should exist");
        assert_eq!(node.definition().semantic_kind(), Some(kind));
        assert_eq!(
            node.definition()
                .choice_options()
                .map(|option| (option.value().clone(), option.label().to_owned()))
                .collect::<Vec<_>>(),
            [(value, label.to_owned())]
        );
        assert_eq!(node.display_text().as_deref(), Some(label));
        assert_eq!(node.allowed_operations(), Default::default());
    }
}

#[test]
fn structured_or_empty_scalar_choices_report_located_capability_findings() {
    for (property_schema, code) in [
        (
            json!({ "enum": ["ok", { "structured": true }] }),
            "validation.enum.structured",
        ),
        (
            json!({ "type": "boolean", "enum": ["yes"] }),
            "validation.enum.incompatible",
        ),
        (
            json!({ "enum": ["yes"], "allOf": [false] }),
            "validation.enum.incompatible",
        ),
    ] {
        let error = match FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": { "value": property_schema }
        })) {
            Ok(_) => panic!("an unrepresentable choice must not compile strictly"),
            Err(error) => error,
        };
        let CompileError::Capability(report) = error else {
            panic!("the unrepresentable choice should produce a capability report");
        };
        assert!(report.findings().any(|finding| {
            finding.code() == code && finding.instance_location().as_str() == "/value"
        }));
    }
}

fn control_with_binding(form: &schemaform::Form, binding: &str) -> InstanceIdentity {
    let mut pending = vec![form.view().root()];
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).expect("the form node should exist");
        if node
            .binding()
            .is_some_and(|current| current.pointer().as_str() == binding)
        {
            return identity;
        }
        pending.extend(node.children());
    }
    panic!("the bound control should exist")
}
