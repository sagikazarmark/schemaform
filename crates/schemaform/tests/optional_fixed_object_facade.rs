use schemaform::{
    ExternalFinding, ExternalFindingBatch, FormDefinition, InstanceIdentity, JsonPointer,
    SubmissionOutcome,
    form::{
        FindingVisibility, FindingVisibilityPolicy, SubmissionBlocker, UserOperationError,
        ValidationOutcomeView,
    },
};
use serde_json::{Value, json};

fn optional_settings_schema(default: Value) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "settings": {
                "type": "object",
                "title": "Settings",
                "default": default,
                "additionalProperties": false,
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "minLength": 3 }
                }
            }
        }
    })
}

#[test]
fn optional_fixed_object_materializes_repairs_removes_and_submits_explicitly() {
    let definition = FormDefinition::compile(optional_settings_schema(json!({ "name": "Li" })))
        .expect("the optional fixed object should compile");
    let mut form = definition
        .create_form(json!({}))
        .expect("the absent optional object should remain constructible");
    let settings = node_with_binding(&form, "/settings");
    let name = node_with_binding(&form, "/settings/name");
    let initial_revisions = (form.view().data_revision(), form.view().state_revision());

    assert_eq!(form.form_data(), &json!({}));
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
    assert!(!form.node(settings).unwrap().is_dirty());
    assert!(
        form.node(settings)
            .unwrap()
            .allowed_operations()
            .can_materialize()
    );
    assert!(
        !form
            .node(settings)
            .unwrap()
            .allowed_operations()
            .can_remove_value()
    );
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        initial_revisions,
        "construction and observation must not apply the annotation default"
    );

    let materialized = form
        .user()
        .materialize(settings)
        .expect("the explicit presence action should materialize the object");
    assert_eq!(form.form_data(), &json!({ "settings": { "name": "Li" } }));
    assert!(materialized.changed().any(|identity| identity == settings));
    assert!(materialized.changed().any(|identity| identity == name));
    assert_eq!(materialized.removed().count(), 0);
    assert!(form.node(settings).unwrap().is_dirty());
    assert!(form.node(name).unwrap().is_dirty());
    assert!(
        form.node(settings)
            .unwrap()
            .allowed_operations()
            .can_remove_value()
    );
    assert!(
        !form
            .node(settings)
            .unwrap()
            .allowed_operations()
            .can_materialize()
    );
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Invalid { findings, .. }
            if findings.iter().any(|finding| finding.code() == "minLength")
    ));

    let after_materialize = (form.view().data_revision(), form.view().state_revision());
    assert_eq!(
        form.user().materialize(settings),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        after_materialize
    );
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));

    let repaired_transition = form
        .user()
        .input_text(name, "Grace")
        .expect("the seeded invalid child should be repairable");
    assert!(
        repaired_transition
            .changed()
            .any(|identity| identity == name)
    );
    assert!(
        repaired_transition
            .changed()
            .any(|identity| identity == settings),
        "a child edit changes the fixed object's aggregate dirty projection"
    );
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
    let repaired = form.prepare_submission();
    let repaired_snapshot = match repaired.outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot,
        SubmissionOutcome::Blocked(_) => panic!("the repaired object should submit"),
    };
    assert_eq!(
        repaired_snapshot.form_data(),
        &json!({ "settings": { "name": "Grace" } })
    );

    let removed = form
        .user()
        .remove_value(settings)
        .expect("the optional fixed object should be removable");
    assert_eq!(form.form_data(), &json!({}));
    assert!(removed.changed().any(|identity| identity == settings));
    assert!(removed.changed().any(|identity| identity == name));
    assert_eq!(removed.removed().count(), 0);
    assert!(!form.node(settings).unwrap().is_dirty());
    assert!(!form.node(name).unwrap().is_dirty());
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );

    let removed_preparation = form.prepare_submission();
    let removed_snapshot = match removed_preparation.outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot,
        SubmissionOutcome::Blocked(_) => panic!("the absent optional object should submit"),
    };
    assert_eq!(removed_snapshot.form_data(), &json!({}));

    let after_remove = (form.view().data_revision(), form.view().state_revision());
    assert_eq!(
        form.user().remove_value(settings),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        after_remove
    );
}

#[test]
fn object_creation_seed_requires_one_semantically_unique_object_default() {
    let cases = [
        (
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "default": "child" }
                }
            }),
            json!({}),
        ),
        (
            json!({
                "type": "object",
                "default": "wrong kind",
                "properties": { "name": { "type": "string" } }
            }),
            json!({}),
        ),
        (
            json!({
                "type": "object",
                "allOf": [
                    { "default": { "count": 1 } },
                    { "default": { "count": 1.0 } }
                ],
                "properties": { "count": { "type": "number" } }
            }),
            json!({ "count": 1 }),
        ),
        (
            json!({
                "type": "object",
                "allOf": [
                    { "default": { "name": "Ada" } },
                    { "default": { "name": "Grace" } }
                ],
                "properties": { "name": { "type": "string" } }
            }),
            json!({}),
        ),
    ];

    for (object_schema, expected_seed) in cases {
        let definition = definition_with_settings(object_schema);
        let mut form = definition
            .create_form(json!({}))
            .expect("the absent optional object should remain unchanged initially");
        let settings = node_with_binding(&form, "/settings");
        form.user()
            .materialize(settings)
            .expect("the explicit action should use the resolved creation seed");
        assert_eq!(&form.form_data()["settings"], &expected_seed);
    }

    let referenced = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "settings": { "$ref": "#/$defs/settings" }
        },
        "$defs": {
            "settings": {
                "type": "object",
                "default": { "name": "Ada" },
                "properties": { "name": { "type": "string" } }
            }
        }
    }))
    .expect("the referenced optional fixed object should compile");
    let mut form = referenced.create_form(json!({})).unwrap();
    let settings = node_with_binding(&form, "/settings");
    form.user().materialize(settings).unwrap();
    assert_eq!(form.form_data(), &json!({ "settings": { "name": "Ada" } }));
}

#[test]
fn definition_fingerprint_tracks_creation_seeds_and_preserved_defaults() {
    let ada = FormDefinition::compile(optional_settings_schema(json!({ "name": "Ada" })))
        .expect("the first object seed should compile");
    let grace = FormDefinition::compile(optional_settings_schema(json!({ "name": "Grace" })))
        .expect("the second object seed should compile");
    assert_ne!(ada.fingerprint(), grace.fingerprint());

    let integer = definition_with_settings(json!({
        "type": "object",
        "default": { "count": 1 },
        "properties": { "count": { "type": "number" } }
    }));
    let decimal = definition_with_settings(json!({
        "type": "object",
        "default": { "count": 1.0 },
        "properties": { "count": { "type": "number" } }
    }));
    assert_eq!(integer.fingerprint(), decimal.fingerprint());

    let ignored_a = definition_with_settings(json!({
        "type": "object",
        "default": "ignored-a",
        "properties": { "name": { "type": "string" } }
    }));
    let ignored_b = definition_with_settings(json!({
        "type": "object",
        "default": "ignored-b",
        "properties": { "name": { "type": "string" } }
    }));
    assert_ne!(ignored_a.fingerprint(), ignored_b.fingerprint());
}

#[test]
fn missing_required_fixed_object_can_be_repaired_explicitly() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["settings"],
        "properties": {
            "settings": {
                "type": "object",
                "properties": { "name": { "type": "string" } }
            }
        }
    }))
    .expect("the required fixed object should compile");
    let mut form = definition
        .create_form(json!({}))
        .expect("invalid missing required data should remain constructible");
    let settings = node_with_binding(&form, "/settings");
    let before = (form.view().data_revision(), form.view().state_revision());

    assert!(
        form.node(settings)
            .unwrap()
            .allowed_operations()
            .can_materialize()
    );
    form.user()
        .materialize(settings)
        .expect("the missing required object should be repairable");
    assert_eq!(form.form_data(), &json!({ "settings": {} }));
    assert_ne!(
        (form.view().data_revision(), form.view().state_revision()),
        before
    );
}

#[test]
fn object_presence_changes_reconcile_descendant_array_identities() {
    let definition = definition_with_settings(json!({
        "type": "object",
        "default": { "tags": ["seed"] },
        "properties": {
            "tags": {
                "type": "array",
                "items": { "type": "string" }
            }
        }
    }));
    let mut form = definition.create_form(json!({})).unwrap();
    let settings = node_with_binding(&form, "/settings");
    let tags = node_with_binding(&form, "/settings/tags");

    let materialized = form.user().materialize(settings).unwrap();
    let first_item = form.node(tags).unwrap().children().next().unwrap();
    assert_eq!(
        form.form_data(),
        &json!({ "settings": { "tags": ["seed"] } })
    );
    assert!(
        materialized
            .changed()
            .any(|identity| identity == first_item)
    );

    form.set_finding_visibility(FindingVisibilityPolicy::new(
        FindingVisibility::Immediate,
        FindingVisibility::Immediate,
    ));
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "row",
            JsonPointer::parse("/settings/tags/0").unwrap(),
            json!({}),
        )],
    ))
    .unwrap();

    let removed = form.user().remove_value(settings).unwrap();
    assert!(form.node(first_item).is_none());
    assert!(removed.removed().any(|identity| identity == first_item));
    assert!(!removed.changed().any(|identity| identity == first_item));

    form.user().materialize(settings).unwrap();
    let second_item = form.node(tags).unwrap().children().next().unwrap();
    assert_ne!(first_item, second_item);

    let mut replaced_form = definition
        .create_form(json!({ "settings": "legacy" }))
        .unwrap();
    let replaced_settings = node_with_binding(&replaced_form, "/settings");
    let replaced_tags = node_with_binding(&replaced_form, "/settings/tags");
    let replaced = replaced_form
        .user()
        .replace_value(replaced_settings, json!({ "tags": ["new"] }))
        .unwrap();
    let replaced_item = replaced_form
        .node(replaced_tags)
        .unwrap()
        .children()
        .next()
        .unwrap();
    assert!(replaced.changed().any(|identity| identity == replaced_item));
}

#[test]
fn removal_discards_descendant_parse_state_before_rematerialization() {
    let definition = definition_with_settings(json!({
        "type": "object",
        "default": { "count": 1 },
        "required": ["count"],
        "properties": { "count": { "type": "number" } }
    }));
    let mut form = definition.create_form(json!({})).unwrap();
    let settings = node_with_binding(&form, "/settings");
    let count = node_with_binding(&form, "/settings/count");
    form.user().materialize(settings).unwrap();

    let blocked = form.user().input_text(count, "not a number").unwrap();
    assert!(form.node(count).unwrap().parse_blocker().is_some());
    assert!(blocked.changed().any(|identity| identity == count));
    assert!(
        !blocked.changed().any(|identity| identity == settings),
        "state-only child edits must not invalidate aggregate object projections"
    );
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Parse { .. }))
    ));

    form.user().remove_value(settings).unwrap();
    assert!(form.node(count).unwrap().parse_blocker().is_none());
    form.user().materialize(settings).unwrap();
    assert_eq!(form.form_data(), &json!({ "settings": { "count": 1 } }));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(_)
    ));
}

fn definition_with_settings(object_schema: Value) -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": { "settings": object_schema }
    }))
    .expect("the optional fixed object should compile")
}

fn node_with_binding(form: &schemaform::Form, binding: &str) -> InstanceIdentity {
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
    panic!("the bound node should exist")
}
