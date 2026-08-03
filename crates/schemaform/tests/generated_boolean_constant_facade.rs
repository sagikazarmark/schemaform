use schemaform::{
    CompileError, FormDefinition, InstanceIdentity, JsonPointer, SubmissionOutcome,
    definition::SemanticKind,
    form::{SubmissionBlocker, UserOperationError, ValidationOutcomeView},
};
use serde_json::{Value, json};

#[test]
fn required_boolean_preserves_false_and_presence_through_edits_and_submission() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["enabled"],
        "properties": {
            "enabled": { "type": "boolean", "title": "Enabled" }
        }
    }))
    .expect("the boolean data schema should compile");
    let mut form = definition
        .create_form(json!({ "enabled": false }))
        .expect("the boolean form should be created");
    let enabled = control_with_binding(&form, "/enabled");
    let node = form
        .node(enabled)
        .expect("the boolean control should exist");

    assert_eq!(
        node.definition().semantic_kind(),
        Some(SemanticKind::Boolean)
    );
    assert_eq!(node.current_data(), Some(&Value::Bool(false)));
    assert_eq!(node.display_text().as_deref(), Some("false"));
    assert!(node.allowed_operations().can_set_value());
    assert!(!node.allowed_operations().can_input_text());
    assert!(!node.is_dirty());

    let transition = form
        .user()
        .set_value(enabled, Value::Bool(true))
        .expect("the checkbox should accept a boolean value");
    assert_eq!(transition.changed().collect::<Vec<_>>(), [enabled]);
    assert_eq!(form.form_data(), &json!({ "enabled": true }));
    assert!(
        form.node(enabled)
            .expect("the control should remain")
            .is_dirty()
    );

    let snapshot = match form.prepare_submission().outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot.clone(),
        SubmissionOutcome::Blocked(_) => panic!("the present boolean should be submittable"),
    };
    assert_eq!(snapshot.form_data(), &json!({ "enabled": true }));

    form.user()
        .set_value(enabled, Value::Bool(false))
        .expect("false should remain a legal required-property value");
    assert_eq!(form.form_data(), &json!({ "enabled": false }));
    assert!(
        !form
            .node(enabled)
            .expect("the control should remain")
            .is_dirty()
    );
    assert_eq!(snapshot.form_data(), &json!({ "enabled": true }));

    assert_eq!(
        form.user().set_value(enabled, json!("false")),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(form.form_data(), &json!({ "enabled": false }));

    let mut missing = definition
        .create_form(json!({}))
        .expect("missing required data should remain constructible");
    let missing_enabled = control_with_binding(&missing, "/enabled");
    let missing_node = missing
        .node(missing_enabled)
        .expect("the missing boolean control should remain represented");
    assert_eq!(missing_node.current_data(), None);
    assert_eq!(missing_node.display_text(), None);
    assert!(matches!(
        missing.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "required")
    ));

    let transition = missing
        .user()
        .set_value(missing_enabled, Value::Bool(false))
        .expect("setting false should materialize the missing required property");
    assert_eq!(transition.changed().collect::<Vec<_>>(), [missing_enabled]);
    assert_eq!(missing.form_data(), &json!({ "enabled": false }));
    let repaired = missing
        .node(missing_enabled)
        .expect("the repaired boolean control should remain represented");
    assert_eq!(repaired.current_data(), Some(&Value::Bool(false)));
    assert!(repaired.is_dirty());
    assert_eq!(
        missing.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );

    let mut incompatible = definition
        .create_form(json!({ "enabled": "yes" }))
        .expect("incompatible boolean data should remain constructible");
    let incompatible_enabled = control_with_binding(&incompatible, "/enabled");
    assert!(matches!(
        incompatible.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "type")
    ));
    assert!(
        !incompatible
            .node(incompatible_enabled)
            .expect("the incompatible control should exist")
            .is_dirty()
    );
    let transition = incompatible
        .user()
        .replace_value(incompatible_enabled, Value::Bool(false))
        .expect("replace value should repair incompatible boolean data explicitly");
    assert_eq!(
        transition.changed().collect::<Vec<_>>(),
        [incompatible_enabled]
    );
    assert_eq!(incompatible.form_data(), &json!({ "enabled": false }));
    assert!(
        incompatible
            .node(incompatible_enabled)
            .expect("the repaired control should exist")
            .is_dirty()
    );
    assert_eq!(
        incompatible.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
}

#[test]
fn scalar_constants_are_fixed_when_compatible_and_repairable_when_incompatible() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["active", "attempts", "region", "unset"],
        "properties": {
            "active": { "title": "Active", "const": true },
            "attempts": { "title": "Attempts", "const": 3 },
            "region": { "title": "Region", "const": "EU" },
            "unset": { "title": "Unset", "const": null }
        }
    }))
    .expect("scalar constants should compile without explicit type declarations");
    let mut form = definition
        .create_form(json!({
            "active": true,
            "attempts": 3,
            "region": "EU",
            "unset": null
        }))
        .expect("the constant form should be created");

    for (binding, expected) in [
        ("/active", "true"),
        ("/attempts", "3"),
        ("/region", "EU"),
        ("/unset", "null"),
    ] {
        let identity = control_with_binding(&form, binding);
        let node = form
            .node(identity)
            .expect("the constant control should exist");
        assert_eq!(
            node.definition().semantic_kind(),
            Some(SemanticKind::Choice)
        );
        assert_eq!(node.display_text().as_deref(), Some(expected));
        assert_eq!(node.allowed_operations(), Default::default());
        assert_eq!(
            form.user().set_value(identity, json!("illegal")),
            Err(UserOperationError::OperationNotAllowed)
        );
    }

    let region = JsonPointer::parse("/region").expect("the region pointer should be valid");
    form.transact(|draft| draft.set(&region, json!("US")))
        .expect("the privileged host should be able to install invalid form data");
    let region_control = control_with_binding(&form, "/region");
    assert_eq!(
        form.node(region_control)
            .expect("the region control should remain")
            .display_text()
            .as_deref(),
        Some("US")
    );
    assert!(
        form.node(region_control)
            .expect("the control should remain")
            .is_dirty()
    );
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));

    form.user()
        .replace_value(region_control, json!("EU"))
        .expect("the fixed value should explicitly repair an incompatible constant");
    assert_eq!(form.form_data()["region"], json!("EU"));

    let snapshot = match form.prepare_submission().outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot.clone(),
        SubmissionOutcome::Blocked(_) => panic!("the restored constants should be submittable"),
    };
    assert_eq!(snapshot.form_data()["region"], json!("EU"));
}

#[test]
fn constants_that_cannot_supply_a_legal_scalar_value_are_explicitly_unsupported() {
    for (property_schema, code) in [
        (
            json!({ "type": "boolean", "const": "yes" }),
            "validation.const.incompatible",
        ),
        (
            json!({ "type": "boolean", "const": { "fixed": true } }),
            "validation.const.structured",
        ),
    ] {
        let error = match FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": { "value": property_schema }
        })) {
            Ok(_) => panic!("a constant with no legal editable value must not compile strictly"),
            Err(error) => error,
        };
        let CompileError::Capability(report) = error else {
            panic!("the incompatible constant should produce a capability report");
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
        let mut children = node.children().collect::<Vec<_>>();
        children.reverse();
        pending.extend(children);
    }
    panic!("the bound control should exist")
}
