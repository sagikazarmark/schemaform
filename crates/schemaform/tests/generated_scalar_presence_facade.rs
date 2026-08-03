use schemaform::{
    CompileError, FormDefinition, InstanceIdentity,
    definition::SemanticKind,
    form::{ScalarValueState, UserOperationError, ValidationOutcomeView},
};
use serde_json::json;

#[test]
fn nullable_string_exposes_all_scalar_states_without_mutating_form_data() {
    let definition = nullable_string_definition();

    for (form_data, expected_state) in [
        (json!({}), ScalarValueState::Missing),
        (json!({ "value": null }), ScalarValueState::Null),
        (json!({ "value": "" }), ScalarValueState::Empty),
        (json!({ "value": "present" }), ScalarValueState::Compatible),
        (json!({ "value": 7 }), ScalarValueState::Incompatible),
    ] {
        let form = definition
            .create_form(form_data.clone())
            .expect("each scalar state should remain constructible");
        let value = control_with_binding(&form, "/value");
        let before = (form.view().data_revision(), form.view().state_revision());
        let node = form.node(value).expect("the scalar control should exist");

        assert!(node.definition().accepts_null());
        assert_eq!(node.value_state(), Some(expected_state));
        assert_eq!(form.form_data(), &form_data);
        assert_eq!(
            (form.view().data_revision(), form.view().state_revision()),
            before
        );
    }
}

#[test]
fn scalar_operations_follow_presence_requiredness_and_compatibility() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["required_text"],
        "properties": {
            "required_text": { "type": ["string", "null"] },
            "optional_boolean": { "type": "boolean" },
            "fixed": { "const": "EU" },
            "nothing": { "type": "null" }
        }
    }))
    .expect("supported scalar presence shapes should compile");
    let form = definition
        .create_form(json!({
            "required_text": 7,
            "optional_boolean": null,
            "fixed": "wrong"
        }))
        .expect("incompatible scalar data should be preserved");

    let required_text = form
        .node(control_with_binding(&form, "/required_text"))
        .expect("the required text control should exist");
    let required_operations = required_text.allowed_operations();
    assert_eq!(
        required_text.value_state(),
        Some(ScalarValueState::Incompatible)
    );
    assert!(!required_operations.can_input_text());
    assert!(!required_operations.can_set_value());
    assert!(required_operations.can_set_null());
    assert!(!required_operations.can_remove_value());
    assert!(required_operations.can_replace_value());

    let optional_boolean = form
        .node(control_with_binding(&form, "/optional_boolean"))
        .expect("the optional boolean control should exist");
    let optional_operations = optional_boolean.allowed_operations();
    assert_eq!(optional_boolean.value_state(), Some(ScalarValueState::Null));
    assert!(!optional_operations.can_set_value());
    assert!(!optional_operations.can_set_null());
    assert!(optional_operations.can_remove_value());
    assert!(optional_operations.can_replace_value());

    let fixed = form
        .node(control_with_binding(&form, "/fixed"))
        .expect("the constant control should exist");
    assert_eq!(fixed.value_state(), Some(ScalarValueState::Incompatible));
    assert!(fixed.allowed_operations().can_replace_value());
    assert!(fixed.allowed_operations().can_remove_value());

    let nothing = form
        .node(control_with_binding(&form, "/nothing"))
        .expect("the null-only control should exist");
    assert_eq!(nothing.value_state(), Some(ScalarValueState::Missing));
    assert!(nothing.allowed_operations().can_set_null());
    assert!(!nothing.allowed_operations().can_remove_value());
}

#[test]
fn explicit_scalar_repairs_recheck_current_legality_at_execution() {
    let definition = nullable_string_definition();
    let mut form = definition
        .create_form(json!({ "value": 7 }))
        .expect("incompatible scalar data should be preserved");
    let value = control_with_binding(&form, "/value");

    form.user()
        .replace_value(value, json!("repaired"))
        .expect("an incompatible scalar should accept explicit replacement");
    assert_eq!(form.form_data(), &json!({ "value": "repaired" }));
    assert_eq!(
        form.node(value)
            .expect("the repaired control should remain")
            .value_state(),
        Some(ScalarValueState::Compatible)
    );

    let before_rejected_replace = (form.view().data_revision(), form.view().state_revision());
    assert_eq!(
        form.user().replace_value(value, json!("stale")),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(form.form_data(), &json!({ "value": "repaired" }));
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        before_rejected_replace
    );

    form.user()
        .set_null(value)
        .expect("a nullable scalar should accept explicit null");
    assert_eq!(form.form_data(), &json!({ "value": null }));

    let before_rejected_null = (form.view().data_revision(), form.view().state_revision());
    assert_eq!(
        form.user().set_null(value),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        before_rejected_null
    );

    form.user()
        .remove_value(value)
        .expect("an optional scalar should be removable");
    assert_eq!(form.form_data(), &json!({}));
    assert_eq!(
        form.node(value)
            .expect("the missing control should remain")
            .value_state(),
        Some(ScalarValueState::Missing)
    );
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );

    let before_rejected_remove = (form.view().data_revision(), form.view().state_revision());
    assert_eq!(
        form.user().remove_value(value),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        before_rejected_remove
    );
}

#[test]
fn missing_and_null_text_are_set_only_by_user_input_and_empty_stays_empty() {
    let definition = nullable_string_definition();

    for initial in [json!({}), json!({ "value": null })] {
        let mut form = definition
            .create_form(initial)
            .expect("the scalar presence state should be constructible");
        let value = control_with_binding(&form, "/value");
        assert!(
            form.node(value)
                .expect("the text control should exist")
                .allowed_operations()
                .can_input_text()
        );

        form.user()
            .input_text(value, "")
            .expect("empty input should explicitly set an empty string");
        assert_eq!(form.form_data(), &json!({ "value": "" }));
        assert_eq!(
            form.node(value)
                .expect("the empty control should remain")
                .value_state(),
            Some(ScalarValueState::Empty)
        );
    }
}

#[test]
fn every_ordinary_scalar_kind_compiles_with_null_and_can_leave_null_explicitly() {
    for (kind, expected_kind, input) in [
        ("string", SemanticKind::String, Some("text")),
        ("number", SemanticKind::Number, Some("1.25")),
        ("integer", SemanticKind::Integer, Some("2")),
        ("boolean", SemanticKind::Boolean, None),
    ] {
        let definition = FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "value": { "type": [kind, "null"] }
            }
        }))
        .expect("one ordinary scalar kind plus null should compile");
        let mut form = definition
            .create_form(json!({ "value": null }))
            .expect("the nullable scalar form should be created");
        let value = control_with_binding(&form, "/value");
        let node = form.node(value).expect("the nullable control should exist");
        assert_eq!(node.definition().semantic_kind(), Some(expected_kind));
        assert!(node.definition().accepts_null());
        assert_eq!(node.value_state(), Some(ScalarValueState::Null));

        if let Some(input) = input {
            form.user()
                .input_text(value, input)
                .expect("text-like input should leave the null state explicitly");
        } else {
            form.user()
                .set_value(value, json!(false))
                .expect("boolean input should leave the null state explicitly");
        }
        assert_eq!(
            form.node(value)
                .expect("the compatible control should remain")
                .value_state(),
            Some(ScalarValueState::Compatible)
        );
        form.user()
            .set_null(value)
            .expect("the nullable scalar should return to null explicitly");
        assert_eq!(form.form_data(), &json!({ "value": null }));
    }
}

#[test]
fn empty_is_distinct_only_when_the_empty_string_is_compatible() {
    let empty_choice = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "value": { "enum": ["", "other"] }
        }
    }))
    .expect("an empty finite scalar choice should compile");
    let choice_form = empty_choice
        .create_form(json!({ "value": "" }))
        .expect("the empty choice form should be created");
    let choice = control_with_binding(&choice_form, "/value");
    assert_eq!(
        choice_form
            .node(choice)
            .expect("the choice control should exist")
            .value_state(),
        Some(ScalarValueState::Empty)
    );

    let boolean = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "value": { "type": "boolean" }
        }
    }))
    .expect("the boolean schema should compile");
    let boolean_form = boolean
        .create_form(json!({ "value": "" }))
        .expect("incompatible empty data should remain constructible");
    let boolean = control_with_binding(&boolean_form, "/value");
    assert_eq!(
        boolean_form
            .node(boolean)
            .expect("the boolean control should exist")
            .value_state(),
        Some(ScalarValueState::Incompatible)
    );
}

#[test]
fn nullable_containers_remain_outside_scalar_presence_support() {
    for (container, property) in [
        (
            "object",
            json!({
                "type": ["object", "null"],
                "properties": {
                    "nested": { "type": "string" }
                }
            }),
        ),
        (
            "array",
            json!({
                "type": ["array", "null"],
                "items": { "type": "string" }
            }),
        ),
    ] {
        let error = match FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": { "value": property }
        })) {
            Ok(_) => panic!("nullable {container} must not compile as a presence control"),
            Err(error) => error,
        };
        let CompileError::Capability(report) = error else {
            panic!("nullable {container} should produce a capability report");
        };
        assert!(report.findings().any(|finding| {
            finding.code() == "validation.type.ambiguous"
                && finding.instance_location().as_str() == "/value"
        }));
    }
}

fn nullable_string_definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "value": { "type": ["string", "null"] }
        }
    }))
    .expect("a nullable string property should compile")
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
