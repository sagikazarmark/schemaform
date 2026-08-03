use schemaform::{
    ExternalFinding, ExternalFindingBatch, Form, FormDefinition, InstanceIdentity, JsonPointer,
    SubmissionOutcome,
    form::{ExternalFindingError, ParseBlockerKind, ReinitializeError, ValidationOutcomeView},
};
use serde_json::{Value, json};

#[test]
fn reset_restores_valid_baseline_from_invalid_parse_blocked_state() {
    let mut form = lifecycle_definition()
        .create_form(json!({ "name": "Ada", "quantity": 1 }))
        .expect("the lifecycle form should be created");
    let name = node_with_binding(&form, "/name");
    let quantity = node_with_binding(&form, "/quantity");

    form.user()
        .input_text(name, "Grace")
        .expect("the name should edit");
    form.user()
        .input_text(quantity, "0")
        .expect("form data invalid against the data schema should remain editable");
    form.user()
        .blur(quantity)
        .expect("the invalid quantity should become touched");
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));
    form.user()
        .input_text(quantity, "-")
        .expect("the incomplete quantity should remain buffered");
    let stale_revision = form.view().data_revision();
    form.apply_external_findings(blocking_batch(stale_revision))
        .expect("a current external finding should apply");
    let before_state = form.view().state_revision();

    let transition = form.reset();

    assert_eq!(form.form_data(), &json!({ "name": "Ada", "quantity": 1 }));
    assert_eq!(transition.before_data_revision(), stale_revision);
    assert_ne!(transition.after_data_revision(), stale_revision);
    assert_eq!(transition.before_state_revision(), before_state);
    assert_ne!(transition.after_state_revision(), before_state);
    assert_eq!(
        transition.after_data_revision(),
        form.view().data_revision()
    );
    assert_eq!(
        transition.after_state_revision(),
        form.view().state_revision()
    );
    assert!(!form.view().submission_attempted());
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    ));
    for identity in [name, quantity] {
        let node = form
            .node(identity)
            .expect("the control should remain present");
        assert_eq!(node.edit_buffer(), None);
        assert_eq!(node.parse_blocker(), None);
        assert!(!node.is_touched());
        assert!(!node.is_dirty());
    }
    assert!(matches!(
        form.apply_external_findings(blocking_batch(stale_revision)),
        Err(ExternalFindingError::StaleRevision { .. })
    ));

    let settled_data = form.view().data_revision();
    let settled_state = form.view().state_revision();
    let settled = form.reset();
    assert!(settled.is_empty());
    assert_eq!(form.view().data_revision(), settled_data);
    assert_eq!(form.view().state_revision(), settled_state);
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(snapshot)
            if snapshot.form_data() == &json!({ "name": "Ada", "quantity": 1 })
    ));
}

#[test]
fn reset_of_semantically_equal_data_clears_interaction_but_preserves_current_findings() {
    let baseline: Value = serde_json::from_str(r#"{"name":"Ada","quantity":1e3}"#)
        .expect("the baseline should parse");
    let mut form = lifecycle_definition()
        .create_form(baseline.clone())
        .expect("the lifecycle form should be created");
    let quantity = node_with_binding(&form, "/quantity");
    let current_revision = form.view().data_revision();

    form.user()
        .input_text(quantity, "1000")
        .expect("the equivalent quantity should parse");
    form.user()
        .blur(quantity)
        .expect("the equivalent quantity should become touched");
    form.apply_external_findings(blocking_batch(current_revision))
        .expect("a current external finding should apply");
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));
    let before_state = form.view().state_revision();

    let transition = form.reset();

    assert_eq!(form.form_data(), &baseline);
    assert_eq!(transition.before_data_revision(), current_revision);
    assert_eq!(transition.after_data_revision(), current_revision);
    assert_ne!(transition.after_state_revision(), before_state);
    let quantity = form
        .node(quantity)
        .expect("the quantity should remain present");
    assert_eq!(quantity.edit_buffer(), None);
    assert_eq!(quantity.parse_blocker(), None);
    assert!(!quantity.is_touched());
    assert!(!quantity.is_dirty());
    assert!(!form.view().submission_attempted());
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                schemaform::form::SubmissionBlocker::External {
                    source,
                    finding,
                } if source == "server"
                    && finding.code() == "server-rejected"
                    && finding.instance_location().as_str() == "/quantity"
            ))
    ));
}

#[test]
fn reinitialize_starts_a_fresh_lifecycle_for_valid_invalid_and_equal_data() {
    let baseline: Value = serde_json::from_str(r#"{"name":"Ada","quantity":1e3}"#)
        .expect("the baseline should parse");
    let mut form = lifecycle_definition()
        .create_form(baseline)
        .expect("the lifecycle form should be created");
    let name = node_with_binding(&form, "/name");
    let quantity = node_with_binding(&form, "/quantity");

    form.user()
        .input_text(quantity, "-")
        .expect("the incomplete quantity should remain buffered");
    form.user()
        .blur(quantity)
        .expect("the incomplete quantity should become touched");
    let rejected_data = form.form_data().clone();
    let rejected_data_revision = form.view().data_revision();
    let rejected_state_revision = form.view().state_revision();
    assert_eq!(
        form.reinitialize(json!([])),
        Err(ReinitializeError::InvalidFormData)
    );
    assert_eq!(form.form_data(), &rejected_data);
    assert_eq!(form.view().data_revision(), rejected_data_revision);
    assert_eq!(form.view().state_revision(), rejected_state_revision);
    assert_eq!(
        form.node(quantity)
            .expect("the quantity should remain present")
            .parse_blocker(),
        Some(ParseBlockerKind::InvalidInteger)
    );

    let valid_transition = form
        .reinitialize(json!({ "name": "Lin", "quantity": 2 }))
        .expect("valid data should start a fresh lifecycle");
    assert_ne!(
        valid_transition.before_data_revision(),
        valid_transition.after_data_revision()
    );
    assert_ne!(
        valid_transition.before_state_revision(),
        valid_transition.after_state_revision()
    );
    assert_settled(&form, [name, quantity]);
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    ));

    let invalid_transition = form
        .reinitialize(json!({ "name": "Lin", "quantity": 0 }))
        .expect("object data invalid against the data schema should remain repairable");
    assert_ne!(
        invalid_transition.before_data_revision(),
        invalid_transition.after_data_revision()
    );
    assert_ne!(
        invalid_transition.before_state_revision(),
        invalid_transition.after_state_revision()
    );
    assert_settled(&form, [name, quantity]);
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "minimum")
    ));

    let revision_before_equal = form.view().data_revision();
    form.apply_external_findings(blocking_batch(revision_before_equal))
        .expect("a finding for the invalid lifecycle should apply");
    form.user()
        .input_text(quantity, "0e3")
        .expect("an equivalent invalid value should remain buffered");
    form.user()
        .blur(quantity)
        .expect("the equivalent invalid value should become touched");
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));

    let equal_transition = form
        .reinitialize(json!({ "name": "Lin", "quantity": 0 }))
        .expect("equal data should still start a fresh lifecycle");
    assert_ne!(
        equal_transition.before_data_revision(),
        equal_transition.after_data_revision()
    );
    assert_ne!(
        equal_transition.before_state_revision(),
        equal_transition.after_state_revision()
    );
    assert_settled(&form, [name, quantity]);
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if !blockers.iter().any(|blocker| matches!(
                blocker,
                schemaform::form::SubmissionBlocker::External { .. }
            ))
    ));
    assert!(matches!(
        form.apply_external_findings(blocking_batch(revision_before_equal)),
        Err(ExternalFindingError::StaleRevision { .. })
    ));

    form.user()
        .input_text(quantity, "3")
        .expect("the invalid lifecycle should remain repairable");
    form.reset();
    assert_eq!(form.form_data(), &json!({ "name": "Lin", "quantity": 0 }));
    assert!(
        !form
            .node(quantity)
            .expect("the quantity should remain present")
            .is_dirty()
    );
}

fn lifecycle_definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "quantity"],
        "properties": {
            "name": { "type": "string" },
            "quantity": { "type": "integer", "minimum": 1 }
        }
    }))
    .expect("the lifecycle data schema should compile")
}

fn blocking_batch(revision: schemaform::DataRevision) -> ExternalFindingBatch {
    ExternalFindingBatch::new(
        "server",
        revision,
        [ExternalFinding::blocking(
            "server-rejected",
            JsonPointer::parse("/quantity").expect("the quantity pointer should be valid"),
            json!({}),
        )],
    )
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
    panic!("the bound node should exist")
}

fn assert_settled<const N: usize>(form: &Form, identities: [InstanceIdentity; N]) {
    assert!(!form.view().submission_attempted());
    for identity in identities {
        let node = form
            .node(identity)
            .expect("the control should remain present");
        assert_eq!(node.edit_buffer(), None);
        assert_eq!(node.parse_blocker(), None);
        assert!(!node.is_touched());
        assert!(!node.is_dirty());
    }
}
