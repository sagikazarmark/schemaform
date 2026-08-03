use std::num::NonZeroUsize;

use schemaform::{
    ExternalFinding, ExternalFindingBatch, FindingView, FindingVisibility, FindingVisibilityPolicy,
    Form, FormDefinition, InstanceIdentity, ItemIdentity, JsonPointer, RetrievalUri,
    SubmissionOutcome,
    form::{
        ExternalFindingLimits, HostCommitError, SubmissionBlocker, UserOperationError,
        ValidationOutcomeView,
    },
};
use serde_json::json;

#[test]
fn arrays_materialize_replace_and_remove_with_deterministic_seeds() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["required", "incompatible"],
        "properties": {
            "required": {
                "type": "array",
                "default": ["defaulted"],
                "items": { "type": "string" }
            },
            "optional": {
                "type": "array",
                "allOf": [
                    { "default": ["first"] },
                    { "default": ["second"] }
                ],
                "items": { "type": "string" }
            },
            "incompatible": {
                "type": "array",
                "items": { "type": "string" }
            }
        }
    }))
    .unwrap();
    let mut form = definition
        .create_form(json!({ "incompatible": 7 }))
        .unwrap();
    let required = node_with_binding(&form, "/required");
    let optional = node_with_binding(&form, "/optional");
    let incompatible = node_with_binding(&form, "/incompatible");

    assert!(
        form.node(required)
            .unwrap()
            .allowed_operations()
            .can_materialize()
    );
    assert!(
        form.node(optional)
            .unwrap()
            .allowed_operations()
            .can_materialize()
    );
    assert!(
        form.node(incompatible)
            .unwrap()
            .allowed_operations()
            .can_replace_value()
    );

    form.user().materialize(required).unwrap();
    form.user().materialize(optional).unwrap();
    form.user().replace_value(incompatible, json!([])).unwrap();
    assert_eq!(
        form.form_data(),
        &json!({
            "required": ["defaulted"],
            "optional": [],
            "incompatible": []
        })
    );
    assert!(
        form.node(optional)
            .unwrap()
            .allowed_operations()
            .can_remove_value()
    );
    form.user().remove_value(optional).unwrap();
    assert_eq!(form.form_data().get("optional"), None);
}

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn scalar_array_append_and_remove_preserve_surviving_item_identity() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["tags"],
        "properties": {
            "tags": {
                "type": "array",
                "title": "Tags",
                "items": {
                    "type": "string",
                    "default": "new tag"
                }
            }
        }
    }))
    .expect("a homogeneous scalar array should compile");
    let mut form = definition
        .create_form(json!({ "tags": ["same", "same"] }))
        .expect("the scalar array form should be created");
    let array = node_with_binding(&form, "/tags");
    let initial = array_items(&form, array);

    assert_eq!(initial.len(), 2);
    assert_ne!(
        initial[0].1, initial[1].1,
        "duplicate values need distinct identity"
    );
    assert_eq!(
        form.node(initial[0].0).unwrap().current_data(),
        Some(&json!("same"))
    );
    assert_eq!(
        form.node(initial[1].0).unwrap().current_data(),
        Some(&json!("same"))
    );
    assert!(
        form.node(array)
            .unwrap()
            .allowed_operations()
            .can_append_item()
    );
    assert!(
        form.node(array)
            .unwrap()
            .allowed_operations()
            .can_remove_item()
    );
    assert!(
        form.node(array)
            .unwrap()
            .allowed_operations()
            .can_move_item()
    );

    let appended = form
        .user()
        .append_item(array)
        .expect("append should use the resolved item seed");
    let after_append = array_items(&form, array);
    assert_eq!(
        form.form_data(),
        &json!({ "tags": ["same", "same", "new tag"] })
    );
    assert_eq!(after_append[0].1, initial[0].1);
    assert_eq!(after_append[1].1, initial[1].1);
    assert_ne!(after_append[2].1, initial[0].1);
    assert_ne!(after_append[2].1, initial[1].1);
    assert!(appended.changed().any(|identity| identity == array));
    assert!(
        appended
            .changed()
            .any(|identity| identity == after_append[2].0)
    );

    let removed_identity = initial[0].0;
    let removed = form
        .user()
        .remove_item(array, initial[0].1)
        .expect("remove should target the opaque item identity");
    let after_remove = array_items(&form, array);
    assert_eq!(form.form_data(), &json!({ "tags": ["same", "new tag"] }));
    assert_eq!(after_remove[0].1, initial[1].1);
    assert_eq!(after_remove[1].1, after_append[2].1);
    assert!(form.node(removed_identity).is_none());
    assert!(
        removed
            .removed()
            .any(|identity| identity == removed_identity)
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn read_only_scalar_arrays_reject_move_operations() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["values"],
        "properties": {
            "values": {
                "type": "array",
                "readOnly": true,
                "items": { "type": "string" }
            }
        }
    }))
    .unwrap();
    let mut form = definition
        .create_form(json!({ "values": ["same", "same"] }))
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let items = array_items(&form, array);
    let before = (form.view().data_revision(), form.view().state_revision());

    assert!(
        !form
            .node(array)
            .unwrap()
            .allowed_operations()
            .can_move_item()
    );
    assert!(form.user().move_item_down(array, items[0].1).is_err());
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision()),
        before
    );
    assert_eq!(array_items(&form, array), items);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn scalar_array_insert_before_targets_duplicate_item_identity() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["tags"],
        "properties": {
            "tags": {
                "type": "array",
                "items": { "type": "string", "default": "new" }
            }
        }
    }))
    .unwrap();
    let mut form = definition
        .create_form(json!({ "tags": ["same", "same"] }))
        .unwrap();
    let array = node_with_binding(&form, "/tags");
    let initial = array_items(&form, array);
    let before = (form.view().data_revision(), form.view().state_revision());

    let transition = form
        .user()
        .insert_item_before(array, initial[1].1)
        .expect("insert should target the second duplicate by identity");

    let inserted = array_items(&form, array);
    assert_eq!(
        form.form_data(),
        &json!({ "tags": ["same", "new", "same"] })
    );
    assert_eq!(inserted[0], initial[0]);
    assert_eq!(inserted[2], initial[1]);
    assert_ne!(inserted[1].1, initial[0].1);
    assert_ne!(inserted[1].1, initial[1].1);
    assert_ne!(transition.after_data_revision(), before.0);
    assert_ne!(transition.after_state_revision(), before.1);
    assert!(transition.changed().any(|identity| identity == array));
    assert!(
        transition
            .changed()
            .any(|identity| identity == inserted[1].0)
    );
    assert!(
        transition
            .changed()
            .any(|identity| identity == initial[1].0)
    );
    assert_eq!(transition.removed().count(), 0);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn array_cardinality_commands_invalidate_a_sibling_when_conditional_findings_change() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values", "label"],
        "properties": {
            "values": {
                "type": "array",
                "items": { "type": "integer", "default": 0 }
            },
            "label": { "type": "string" }
        },
        "if": {
            "properties": { "values": { "minItems": 2 } },
            "required": ["values"]
        },
        "then": {
            "properties": { "label": { "minLength": 1 } }
        }
    }))
    .expect("the constraint-only cardinality conditional should compile");
    let mut form = definition
        .form(json!({ "values": [0], "label": "" }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::Immediate,
        ))
        .build()
        .expect("the initially valid form should be created");
    let array = node_with_binding(&form, "/values");
    let label = node_with_binding(&form, "/label");

    let appended = form.user().append_item(array).unwrap();
    assert!(appended.changed().any(|identity| identity == label));
    assert!(
        form.node(label)
            .unwrap()
            .validation_findings()
            .any(|finding| { finding.code() == "minLength" })
    );

    let appended_item = array_items(&form, array)[1].1;
    let removed = form.user().remove_item(array, appended_item).unwrap();
    assert!(removed.changed().any(|identity| identity == label));
    assert_eq!(form.node(label).unwrap().validation_findings().count(), 0);

    let first = array_items(&form, array)[0].1;
    let inserted = form.user().insert_item_before(array, first).unwrap();
    assert!(inserted.changed().any(|identity| identity == label));
    assert!(
        form.node(label)
            .unwrap()
            .validation_findings()
            .any(|finding| { finding.code() == "minLength" })
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn data_changing_moves_invalidate_a_sibling_when_conditional_findings_change() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values", "label"],
        "properties": {
            "values": {
                "type": "array",
                "items": { "type": "integer" }
            },
            "label": { "type": "string" }
        },
        "if": {
            "properties": {
                "values": { "prefixItems": [{ "const": 1 }] }
            },
            "required": ["values"]
        },
        "then": {
            "properties": { "label": { "minLength": 1 } }
        }
    }))
    .expect("the constraint-only order conditional should compile");
    let mut form = definition
        .form(json!({ "values": [0, 1], "label": "" }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::Immediate,
        ))
        .build()
        .expect("the initially valid form should be created");
    let array = node_with_binding(&form, "/values");
    let label = node_with_binding(&form, "/label");
    let second = array_items(&form, array)[1].1;

    let moved_up = form.user().move_item_up(array, second).unwrap();
    assert!(moved_up.changed().any(|identity| identity == label));
    assert!(
        form.node(label)
            .unwrap()
            .validation_findings()
            .any(|finding| { finding.code() == "minLength" })
    );

    let moved_down = form.user().move_item_down(array, second).unwrap();
    assert!(moved_down.changed().any(|identity| identity == label));
    assert_eq!(form.node(label).unwrap().validation_findings().count(), 0);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn item_identities_from_another_form_are_rejected_atomically() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["values"],
        "properties": {
            "values": {
                "type": "array",
                "items": { "type": "string", "default": "new" }
            }
        }
    }))
    .unwrap();
    let mut form = definition
        .create_form(json!({ "values": ["first", "second"] }))
        .unwrap();
    let other = definition
        .create_form(json!({ "values": ["other", "other"] }))
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let other_array = node_with_binding(&other, "/values");
    let foreign = array_items(&other, other_array)[0].1;
    let data_before = form.form_data().clone();
    let revisions_before = (form.view().data_revision(), form.view().state_revision());
    let identities_before = array_items(&form, array);

    for result in [
        form.user().insert_item_before(array, foreign),
        form.user().remove_item(array, foreign),
        form.user().move_item_up(array, foreign),
        form.user().move_item_down(array, foreign),
    ] {
        assert_eq!(result.unwrap_err(), UserOperationError::UnknownTarget);
        assert_eq!(form.form_data(), &data_before);
        assert_eq!(
            (form.view().data_revision(), form.view().state_revision()),
            revisions_before
        );
        assert_eq!(array_items(&form, array), identities_before);
    }

    let pointer = JsonPointer::parse("/values").unwrap();
    let first_value = JsonPointer::parse("/values/0").unwrap();
    for result in [
        form.transact(|draft| {
            draft.set(&first_value, json!("changed"));
            draft.insert_item_before(&pointer, foreign, json!("inserted"));
        }),
        form.transact(|draft| {
            draft.set(&first_value, json!("changed"));
            draft.remove_item(&pointer, foreign);
        }),
        form.transact(|draft| {
            draft.set(&first_value, json!("changed"));
            draft.move_item_up(&pointer, foreign);
        }),
        form.transact(|draft| {
            draft.set(&first_value, json!("changed"));
            draft.move_item_down(&pointer, foreign);
        }),
    ] {
        assert_eq!(result.unwrap_err(), HostCommitError::InvalidOperation);
        assert_eq!(form.form_data(), &data_before);
        assert_eq!(
            (form.view().data_revision(), form.view().state_revision()),
            revisions_before
        );
        assert_eq!(array_items(&form, array), identities_before);
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn moving_duplicate_items_preserves_logical_item_state_and_data_revision() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["values"],
        "properties": {
            "values": {
                "type": "array",
                "items": { "type": "number" }
            }
        }
    }))
    .unwrap();
    let mut form = definition
        .form(json!({ "values": [1, 1] }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::Immediate,
        ))
        .build()
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let initial = array_items(&form, array);
    let logical_item = initial[1];
    form.user()
        .input_text(logical_item.0, "not a number")
        .unwrap();
    form.user().blur(logical_item.0).unwrap();
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "review",
            JsonPointer::parse("/values/1").unwrap(),
            json!({}),
        )],
    ))
    .unwrap();
    let before_move = (form.view().data_revision(), form.view().state_revision());

    let moved_up = form
        .user()
        .move_item_up(array, logical_item.1)
        .expect("the second duplicate should move by identity");

    assert_eq!(form.form_data(), &json!({ "values": [1, 1] }));
    assert_eq!(form.view().data_revision(), before_move.0);
    assert_ne!(form.view().state_revision(), before_move.1);
    assert_eq!(array_items(&form, array), vec![logical_item, initial[0]]);
    let moved = form.node(logical_item.0).unwrap();
    assert_eq!(moved.binding().unwrap().pointer().as_str(), "/values/0");
    assert_eq!(moved.edit_buffer(), Some("not a number"));
    assert!(moved.parse_blocker().is_some());
    assert!(moved.is_touched());
    assert_eq!(
        moved
            .external_findings()
            .next()
            .unwrap()
            .1
            .instance_location()
            .as_str(),
        "/values/0"
    );
    assert!(moved_up.changed().any(|identity| identity == array));
    assert!(
        moved_up
            .changed()
            .any(|identity| identity == logical_item.0)
    );
    assert!(moved_up.changed().any(|identity| identity == initial[0].0));
    assert_eq!(moved_up.removed().count(), 0);
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Parse { target, .. } if *target == logical_item.0
            )) && blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::External { source, finding }
                    if source == "server"
                        && finding.instance_location().as_str() == "/values/0"
            ))
    ));

    form.user()
        .move_item_down(array, logical_item.1)
        .expect("the same logical item should move down by identity");
    assert_eq!(array_items(&form, array), initial);
    let restored = form.node(logical_item.0).unwrap();
    assert_eq!(restored.binding().unwrap().pointer().as_str(), "/values/1");
    assert_eq!(restored.edit_buffer(), Some("not a number"));
    assert_eq!(restored.external_findings().count(), 1);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn moving_unequal_items_changes_data_revision_and_rejects_boundaries_atomically() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["values"],
        "properties": {
            "values": {
                "type": "array",
                "maxItems": 1,
                "items": { "type": "integer" }
            }
        }
    }))
    .unwrap();
    let mut form = definition.create_form(json!({ "values": [1, 2] })).unwrap();
    let array = node_with_binding(&form, "/values");
    let initial = array_items(&form, array);
    let before_rejection = (form.view().data_revision(), form.view().state_revision());

    assert!(form.user().move_item_up(array, initial[0].1).is_err());
    assert!(form.user().move_item_down(array, initial[1].1).is_err());
    assert_eq!(
        (form.view().data_revision(), form.view().state_revision(),),
        before_rejection
    );
    assert_eq!(array_items(&form, array), initial);

    let moved = form
        .user()
        .move_item_down(array, initial[0].1)
        .expect("moving remains legal when array length exceeds maxItems");
    assert_eq!(form.form_data(), &json!({ "values": [2, 1] }));
    assert_ne!(moved.after_data_revision(), before_rejection.0);
    assert_ne!(moved.after_state_revision(), before_rejection.1);
    assert_eq!(array_items(&form, array), vec![initial[1], initial[0]]);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn scalar_array_length_bounds_reject_only_crossing_or_worsening_changes() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["values"],
        "properties": {
            "values": {
                "type": "array",
                "minItems": 2,
                "maxItems": 3,
                "items": { "type": "integer" }
            }
        }
    }))
    .expect("bounded homogeneous arrays should compile");

    let mut below_minimum = definition.create_form(json!({ "values": [1] })).unwrap();
    let array = node_with_binding(&below_minimum, "/values");
    let only_item = array_items(&below_minimum, array)[0].1;
    let before_rejection = (
        below_minimum.view().data_revision(),
        below_minimum.view().state_revision(),
    );
    assert!(
        below_minimum
            .node(array)
            .unwrap()
            .allowed_operations()
            .can_append_item()
    );
    assert!(
        !below_minimum
            .node(array)
            .unwrap()
            .allowed_operations()
            .can_remove_item()
    );
    assert!(below_minimum.user().remove_item(array, only_item).is_err());
    assert_eq!(
        (
            below_minimum.view().data_revision(),
            below_minimum.view().state_revision(),
        ),
        before_rejection
    );
    below_minimum.user().append_item(array).unwrap();
    assert_eq!(below_minimum.form_data(), &json!({ "values": [1, 0] }));

    let mut at_maximum = definition
        .create_form(json!({ "values": [1, 2, 3] }))
        .unwrap();
    let array = node_with_binding(&at_maximum, "/values");
    let before_rejection = (
        at_maximum.view().data_revision(),
        at_maximum.view().state_revision(),
    );
    assert!(
        !at_maximum
            .node(array)
            .unwrap()
            .allowed_operations()
            .can_append_item()
    );
    assert!(
        at_maximum
            .node(array)
            .unwrap()
            .allowed_operations()
            .can_remove_item()
    );
    assert!(at_maximum.user().append_item(array).is_err());
    let first = array_items(&at_maximum, array)[0].1;
    assert!(at_maximum.user().insert_item_before(array, first).is_err());
    assert_eq!(
        (
            at_maximum.view().data_revision(),
            at_maximum.view().state_revision()
        ),
        before_rejection
    );

    let mut above_maximum = definition
        .create_form(json!({ "values": [1, 2, 3, 4] }))
        .unwrap();
    let array = node_with_binding(&above_maximum, "/values");
    let first = array_items(&above_maximum, array)[0].1;
    assert!(
        !above_maximum
            .node(array)
            .unwrap()
            .allowed_operations()
            .can_append_item()
    );
    assert!(
        above_maximum
            .node(array)
            .unwrap()
            .allowed_operations()
            .can_remove_item()
    );
    above_maximum.user().remove_item(array, first).unwrap();
    assert_eq!(above_maximum.form_data(), &json!({ "values": [2, 3, 4] }));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn mathematical_array_length_bounds_gate_structural_operations() {
    let data_schema = serde_json::from_str(
        r#"{
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "values": {
                    "type": "array",
                    "minItems": 2.0,
                    "maxItems": 3e0,
                    "items": { "type": "integer" }
                }
            }
        }"#,
    )
    .expect("the mathematically integral bounds should parse");
    let definition = FormDefinition::compile(data_schema)
        .expect("the mathematically integral bounds should compile");
    let mut form = definition
        .create_form(json!({ "values": [1, 2] }))
        .expect("the bounded array form should be created");
    let array = node_with_binding(&form, "/values");
    let first = array_items(&form, array)[0].1;

    assert!(
        !form
            .node(array)
            .unwrap()
            .allowed_operations()
            .can_remove_item()
    );
    assert_eq!(
        form.user().remove_item(array, first),
        Err(UserOperationError::OperationNotAllowed)
    );

    form.user()
        .append_item(array)
        .expect("appending up to maxItems should remain legal");
    assert!(
        !form
            .node(array)
            .unwrap()
            .allowed_operations()
            .can_append_item()
    );
    assert_eq!(
        form.user().append_item(array),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert_eq!(form.form_data(), &json!({ "values": [1, 2, 0] }));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn array_length_bounds_larger_than_usize_remain_enforced() {
    let data_schema = serde_json::from_str(
        r##"{
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "minimum": { "$ref": "#/x-opaque/const" },
                "z-maximum": { "$ref": "#/x-opaque/default" }
            },
            "x-opaque": {
                "$id": "https://schemas.example/opaque-oversized-bounds",
                "const": {
                    "type": "array",
                    "minItems": 1e4096,
                    "items": { "type": "integer" }
                },
                "default": {
                    "type": "array",
                    "maxItems": 1e4096,
                    "items": { "type": "integer" }
                }
            }
        }"##,
    )
    .expect("the arbitrary-precision array bound should parse");
    let definition = FormDefinition::compiler(data_schema)
        .root_uri(RetrievalUri::parse("https://schemas.example/root.json").unwrap())
        .compile()
        .expect("the arbitrary-precision array bound should compile");
    let mut form = definition
        .create_form(json!({ "minimum": [1], "z-maximum": [1] }))
        .expect("the structurally valid array should remain editable");
    let minimum = node_with_binding(&form, "/minimum");
    let maximum = node_with_binding(&form, "/z-maximum");
    let first = array_items(&form, minimum)[0].1;

    let view = form.view();
    let ValidationOutcomeView::Invalid {
        findings,
        truncated: false,
    } = view.validation_outcome()
    else {
        panic!("the oversized minItems assertion should reject every realizable array")
    };
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code(), "minItems");
    assert_eq!(
        findings[0].keyword_location().pointer().as_str(),
        "/const/minItems"
    );
    assert_eq!(
        findings[0].keyword_location().resource().as_str(),
        "https://schemas.example/opaque-oversized-bounds"
    );
    assert_eq!(
        findings[0].parameters(),
        &serde_json::from_str::<serde_json::Value>(r#"{"limit":1e4096}"#)
            .expect("the expected arbitrary-precision parameter should parse")
    );

    assert!(
        !form
            .node(minimum)
            .unwrap()
            .allowed_operations()
            .can_remove_item()
    );
    assert_eq!(
        form.user().remove_item(minimum, first),
        Err(UserOperationError::OperationNotAllowed)
    );
    assert!(
        form.node(maximum)
            .unwrap()
            .allowed_operations()
            .can_append_item()
    );
    form.user()
        .append_item(maximum)
        .expect("an oversized maxItems bound should not reject a realizable append");
    assert_eq!(
        form.form_data(),
        &json!({ "minimum": [1], "z-maximum": [1, 0] })
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn custom_cardinality_keywords_do_not_rewrite_instance_data() {
    let huge_bound = serde_json::from_str::<serde_json::Value>("1e4096")
        .expect("the arbitrary-precision number should parse");
    let minimum_schema = json!({
        "type": "array",
        "minItems": huge_bound.clone(),
        "items": { "type": "integer" }
    });
    let maximum_schema = json!({
        "type": "array",
        "maxItems": huge_bound,
        "items": { "type": "integer" }
    });
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["configuration", "selection", "minimum", "maximum"],
        "properties": {
            "configuration": {
                "const": minimum_schema.clone()
            },
            "selection": {
                "enum": [maximum_schema.clone()]
            },
            "minimum": { "$ref": "#/properties/configuration/const" },
            "maximum": { "$ref": "#/properties/selection/enum/0" }
        }
    });
    let definition = FormDefinition::compiler(data_schema)
        .analyze()
        .expect("the data schema should remain analyzable")
        .into_parts()
        .0;
    let form = definition
        .create_form(json!({
            "configuration": minimum_schema,
            "selection": maximum_schema,
            "minimum": [1],
            "maximum": [1]
        }))
        .expect("the dual-role instance values should remain intact");

    let view = form.view();
    let ValidationOutcomeView::Invalid {
        findings,
        truncated: false,
    } = view.validation_outcome()
    else {
        panic!("the oversized minimum should be the only invalid assertion")
    };
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code(), "minItems");
    assert_eq!(findings[0].instance_location().as_str(), "/minimum");
    assert_eq!(
        findings[0].keyword_location().pointer().as_str(),
        "/properties/configuration/const/minItems"
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn array_assertions_produce_structured_findings_and_never_rewrite_moves() {
    struct ArrayAssertionCase {
        assertion: &'static str,
        array_data_schema: serde_json::Value,
        form_data: serde_json::Value,
        finding_code: &'static str,
        keyword_pointer: &'static str,
        parameters: serde_json::Value,
    }

    let cases = [
        ArrayAssertionCase {
            assertion: "uniqueItems",
            array_data_schema: json!({
                "type": "array",
                "uniqueItems": true,
                "items": { "type": "integer" }
            }),
            form_data: json!([1, 1]),
            finding_code: "uniqueItems",
            keyword_pointer: "/properties/values/uniqueItems",
            parameters: json!({}),
        },
        ArrayAssertionCase {
            assertion: "contains",
            array_data_schema: json!({
                "type": "array",
                "contains": { "minimum": 2 },
                "items": { "type": "integer" }
            }),
            form_data: json!([0, 1]),
            finding_code: "contains",
            keyword_pointer: "/properties/values/contains",
            parameters: json!({}),
        },
        ArrayAssertionCase {
            assertion: "minContains",
            array_data_schema: json!({
                "type": "array",
                "contains": { "minimum": 2 },
                "minContains": 2,
                "items": { "type": "integer" }
            }),
            form_data: json!([0, 2]),
            finding_code: "minContains",
            keyword_pointer: "/properties/values/minContains",
            parameters: json!({ "limit": 2 }),
        },
        ArrayAssertionCase {
            assertion: "maxContains",
            array_data_schema: json!({
                "type": "array",
                "contains": { "minimum": 2 },
                "maxContains": 1,
                "items": { "type": "integer" }
            }),
            form_data: json!([2, 3]),
            finding_code: "maxContains",
            keyword_pointer: "/properties/values/maxContains",
            parameters: json!({ "limit": 1 }),
        },
    ];

    for case in cases {
        let definition = FormDefinition::compiler(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["values"],
            "properties": { "values": case.array_data_schema }
        }))
        .root_uri(RetrievalUri::parse("urn:schemaform:test:array-assertions").unwrap())
        .compile()
        .expect("the array assertion should compile");
        let mut form = definition
            .create_form(json!({ "values": case.form_data.clone() }))
            .expect("data-schema-invalid array form data should remain constructible");

        let view = form.view();
        let findings = match view.validation_outcome() {
            ValidationOutcomeView::Invalid {
                findings,
                truncated: false,
            } => findings,
            _ => panic!(
                "{} should produce an invalid validation outcome",
                case.assertion
            ),
        };
        assert_eq!(
            findings.len(),
            1,
            "unexpected findings for {}",
            case.assertion
        );
        let finding = &findings[0];
        assert_eq!(finding.instance_location().as_str(), "/values");
        assert_eq!(
            finding.keyword_location().resource().as_str(),
            "urn:schemaform:test:array-assertions"
        );
        assert_eq!(
            finding.keyword_location().pointer().as_str(),
            case.keyword_pointer
        );
        assert_eq!(finding.code(), case.finding_code);
        assert_eq!(finding.parameters(), &case.parameters);
        assert!(matches!(
            form.prepare_submission().outcome(),
            SubmissionOutcome::Blocked(blockers)
                if blockers
                    .iter()
                    .any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
        ));

        let array = node_with_binding(&form, "/values");
        let items = array_items(&form, array);
        form.user()
            .move_item_down(array, items[0].1)
            .expect("validation must not reject or reinterpret an explicit move");
        assert_eq!(
            form.form_data(),
            &json!({
                "values": [case.form_data[1].clone(), case.form_data[0].clone()]
            })
        );
        assert!(matches!(
            form.view().validation_outcome(),
            ValidationOutcomeView::Invalid { .. }
        ));
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn referenced_contains_bound_finding_preserves_keyword_and_declared_limit() {
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": { "$ref": "#/x-opaque/array-assertion" }
        },
        "x-opaque": {
            "array-assertion": {
                "type": "array",
                "contains": { "const": 9 },
                "minContains": 2.0,
                "items": { "type": "integer" }
            }
        }
    }))
    .root_uri(RetrievalUri::parse("urn:schemaform:test:opaque-contains").unwrap())
    .compile()
    .expect("an opaque-pointer array assertion should compile");
    let form = definition
        .create_form(json!({ "values": [9] }))
        .expect("data-schema-invalid array form data should remain constructible");

    let view = form.view();
    let ValidationOutcomeView::Invalid { findings, .. } = view.validation_outcome() else {
        panic!("the referenced minContains assertion should reject one match")
    };
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].code(), "minContains");
    assert_eq!(
        findings[0].keyword_location().pointer().as_str(),
        "/x-opaque/array-assertion/minContains"
    );
    assert_eq!(findings[0].parameters(), &json!({ "limit": 2.0 }));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn referenced_array_length_bound_findings_preserve_declared_limits() {
    let data_schema = serde_json::from_str(
        r##"{
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "required": ["minimum", "maximum"],
            "properties": {
                "minimum": { "$ref": "#/x-opaque/const" },
                "maximum": { "$ref": "#/x-opaque/default" }
            },
            "x-opaque": {
                "const": {
                    "type": "array",
                    "minItems": 2.0,
                    "items": { "type": "integer" }
                },
                "default": {
                    "type": "array",
                    "maxItems": 1e0,
                    "items": { "type": "integer" }
                }
            }
        }"##,
    )
    .expect("the referenced array length assertions should parse");
    let definition = FormDefinition::compiler(data_schema)
        .root_uri(RetrievalUri::parse("urn:schemaform:test:opaque-array-length").unwrap())
        .compile()
        .expect("the referenced array length assertions should compile");
    let form = definition
        .create_form(json!({ "minimum": [1], "maximum": [1, 2] }))
        .expect("data-schema-invalid array form data should remain constructible");

    let view = form.view();
    let ValidationOutcomeView::Invalid { findings, .. } = view.validation_outcome() else {
        panic!("the referenced array length assertions should reject the form data")
    };
    assert_eq!(findings.len(), 2);

    let minimum = findings
        .iter()
        .find(|finding| finding.code() == "minItems")
        .expect("the minItems finding should be retained");
    assert_eq!(
        minimum.keyword_location().pointer().as_str(),
        "/x-opaque/const/minItems"
    );
    assert_eq!(
        minimum.parameters(),
        &serde_json::from_str::<serde_json::Value>(r#"{"limit":2.0}"#)
            .expect("the expected decimal minItems parameter should parse")
    );

    let maximum = findings
        .iter()
        .find(|finding| finding.code() == "maxItems")
        .expect("the maxItems finding should be retained");
    assert_eq!(
        maximum.keyword_location().pointer().as_str(),
        "/x-opaque/default/maxItems"
    );
    assert_eq!(
        maximum.parameters(),
        &serde_json::from_str::<serde_json::Value>(r#"{"limit":1e0}"#)
            .expect("the expected scientific maxItems parameter should parse")
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn invalid_contains_array_commands_are_exact_until_a_host_repairs_form_data() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": {
                "type": "array",
                "contains": { "const": 9 },
                "minContains": 2,
                "maxContains": 2,
                "items": { "type": "integer", "default": 0 }
            }
        }
    }))
    .expect("the bounded contains assertion should compile");
    let mut form = definition
        .create_form(json!({ "values": [1, 2] }))
        .expect("data-schema-invalid array form data should remain constructible");
    let array = node_with_binding(&form, "/values");
    let initial = array_items(&form, array);

    form.user().append_item(array).unwrap();
    assert_eq!(form.form_data(), &json!({ "values": [1, 2, 0] }));

    form.user().insert_item_before(array, initial[0].1).unwrap();
    assert_eq!(form.form_data(), &json!({ "values": [0, 1, 2, 0] }));

    form.user().move_item_down(array, initial[0].1).unwrap();
    assert_eq!(form.form_data(), &json!({ "values": [0, 2, 1, 0] }));

    form.user().move_item_up(array, initial[0].1).unwrap();
    assert_eq!(form.form_data(), &json!({ "values": [0, 1, 2, 0] }));

    form.user().remove_item(array, initial[1].1).unwrap();
    assert_eq!(form.form_data(), &json!({ "values": [0, 1, 0] }));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));

    form.transact(|draft| {
        draft.append_item(&JsonPointer::parse("/values").unwrap(), json!(4));
    })
    .expect("a host structural command should not be rewritten by validation");
    assert_eq!(form.form_data(), &json!({ "values": [0, 1, 0, 4] }));

    form.transact(|draft| {
        draft.set(&JsonPointer::parse("/values").unwrap(), json!([9, 9]));
    })
    .expect("the host should be able to repair invalid array form data");
    let preparation = form.prepare_submission();
    let SubmissionOutcome::Ready(snapshot) = preparation.outcome() else {
        panic!("host-repaired array form data should be submittable")
    };
    assert_eq!(snapshot.form_data(), &json!({ "values": [9, 9] }));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn invalid_unique_array_remains_editable_until_the_user_repairs_it() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": {
                "type": "array",
                "uniqueItems": true,
                "items": { "type": "integer" }
            }
        }
    }))
    .expect("the unique array should compile");
    let mut form = definition
        .create_form(json!({ "values": [1, 1, 2] }))
        .expect("an invalid unique array should remain constructible");
    let array = node_with_binding(&form, "/values");
    let items = array_items(&form, array);

    form.user()
        .remove_item(array, items[2].1)
        .expect("removal must target only the requested item");
    assert_eq!(form.form_data(), &json!({ "values": [1, 1] }));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));

    form.user()
        .input_text(items[1].0, "3")
        .expect("a duplicate item should remain editable");
    assert_eq!(form.form_data(), &json!({ "values": [1, 3] }));
    assert!(matches!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    ));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Ready(_)
    ));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn removal_shifts_surviving_item_state_findings_and_submission_by_identity() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "tags"],
        "properties": {
            "name": { "type": "string" },
            "tags": {
                "type": "array",
                "items": { "type": "string", "minLength": 3 }
            }
        }
    }))
    .expect("constrained scalar items should compile");
    let mut form = definition
        .form(json!({ "name": "Ada", "tags": ["first", "bad"] }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::Immediate,
        ))
        .build()
        .unwrap();
    let array = node_with_binding(&form, "/tags");
    let name = node_with_binding(&form, "/name");
    let items = array_items(&form, array);
    let survivor = items[1];

    form.user().blur(survivor.0).unwrap();
    form.user().input_text(survivor.0, "x").unwrap();
    assert!(form.node(survivor.0).unwrap().is_touched());
    assert_eq!(form.node(survivor.0).unwrap().edit_buffer(), Some("x"));
    assert!(
        form.node(survivor.0)
            .unwrap()
            .validation_findings()
            .any(|finding| finding.code() == "minLength")
    );
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [
            ExternalFinding::blocking("taken", JsonPointer::parse("/tags/1").unwrap(), json!({})),
            ExternalFinding::advisory(
                "review-name",
                JsonPointer::parse("/name").unwrap(),
                json!({}),
            ),
        ],
    ))
    .unwrap();
    assert_eq!(
        form.node(survivor.0).unwrap().external_findings().count(),
        1
    );
    let removed = form.user().remove_item(array, items[0].1).unwrap();
    assert!(removed.changed().any(|identity| identity == survivor.0));
    assert!(removed.changed().any(|identity| identity == name));
    let shifted = form
        .node(survivor.0)
        .expect("the surviving identity should remain valid");
    assert_eq!(shifted.binding().unwrap().pointer().as_str(), "/tags/0");
    assert_eq!(shifted.current_data(), Some(&json!("x")));
    assert_eq!(shifted.edit_buffer(), Some("x"));
    assert!(shifted.is_touched());
    assert_eq!(shifted.external_findings().count(), 0);
    assert!(shifted.validation_findings().any(|finding| {
        finding.code() == "minLength" && finding.instance_location().as_str() == "/tags/0"
    }));
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));

    form.user().input_text(survivor.0, "valid").unwrap();
    let preparation = form.prepare_submission();
    let SubmissionOutcome::Ready(snapshot) = preparation.outcome() else {
        panic!("repairing the surviving item should make the array submittable")
    };
    assert_eq!(
        snapshot.form_data(),
        &json!({ "name": "Ada", "tags": ["valid"] })
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn scalar_array_append_uses_each_supported_scalar_seed() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["strings", "integers", "numbers", "booleans", "choices", "nullable", "nullable_conflicting", "null_choice", "nulls"],
        "properties": {
            "strings": {
                "type": "array",
                "items": {
                    "type": "string",
                    "allOf": [
                        { "default": "left" },
                        { "default": "right" }
                    ]
                }
            },
            "integers": { "type": "array", "items": { "type": "integer", "default": 5 } },
            "numbers": { "type": "array", "items": { "type": "number", "default": 1.5 } },
            "booleans": { "type": "array", "items": { "type": "boolean" } },
            "choices": { "type": "array", "items": { "enum": ["b", "a"] } },
            "nullable": {
                "type": "array",
                "items": { "type": ["string", "null"], "default": null }
            },
            "nullable_conflicting": {
                "type": "array",
                "items": {
                    "type": ["string", "null"],
                    "allOf": [
                        { "default": null },
                        { "default": "declared" }
                    ]
                }
            },
            "null_choice": { "type": "array", "items": { "enum": [null] } },
            "nulls": { "type": "array", "items": { "type": "null" } }
        }
    }))
    .expect("all supported scalar array item kinds should compile");
    let mut form = definition
        .create_form(json!({
            "strings": [],
            "integers": [],
            "numbers": [],
            "booleans": [],
            "choices": [],
            "nullable": [],
            "nullable_conflicting": [],
            "null_choice": [],
            "nulls": []
        }))
        .unwrap();

    for binding in [
        "/strings",
        "/integers",
        "/numbers",
        "/booleans",
        "/choices",
        "/nullable",
        "/nullable_conflicting",
        "/null_choice",
        "/nulls",
    ] {
        let array = node_with_binding(&form, binding);
        form.user().append_item(array).unwrap();
    }

    assert_eq!(
        form.form_data(),
        &json!({
            "strings": [""],
            "integers": [5],
            "numbers": [1.5],
            "booleans": [false],
            "choices": ["a"],
            "nullable": [null],
            "nullable_conflicting": [""],
            "null_choice": [null],
            "nulls": [null]
        })
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn scalar_array_parse_blockers_are_visible_and_block_submission() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["values"],
        "properties": {
            "values": { "type": "array", "items": { "type": "number" } }
        }
    }))
    .unwrap();
    let mut form = definition.create_form(json!({ "values": [1] })).unwrap();
    let array = node_with_binding(&form, "/values");
    let item = array_items(&form, array)[0].0;

    form.user().input_text(item, "not a number").unwrap();

    assert!(
        form.view()
            .visible_findings()
            .any(|finding| matches!(finding, FindingView::Parse { target, .. } if target == item))
    );
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn reset_restores_baseline_identity_topology_for_semantically_equal_data() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["values"],
        "properties": {
            "values": {
                "type": "array",
                "items": { "type": "string", "default": "same" }
            }
        }
    }))
    .unwrap();
    let mut form = definition
        .create_form(json!({ "values": ["same", "same"] }))
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let baseline = array_items(&form, array);

    form.user().remove_item(array, baseline[0].1).unwrap();
    form.user().append_item(array).unwrap();
    let replacement = array_items(&form, array)[1];
    assert_eq!(form.form_data(), &json!({ "values": ["same", "same"] }));
    assert_ne!(replacement.1, baseline[0].1);

    let reset = form.reset();
    let restored = array_items(&form, array);
    assert_eq!(restored[0].1, baseline[0].1);
    assert_eq!(restored[1].1, baseline[1].1);
    assert!(reset.changed().any(|identity| identity == baseline[1].0));
    assert!(form.node(baseline[0].0).is_some());
    assert!(form.node(replacement.0).is_none());
    assert!(reset.removed().any(|identity| identity == replacement.0));
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn host_item_edit_preserves_array_item_identities() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["values"],
        "properties": {
            "values": { "type": "array", "items": { "type": "string" } }
        }
    }))
    .unwrap();
    let mut form = definition
        .create_form(json!({ "values": ["first", "second"] }))
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let before = array_items(&form, array);

    form.transact(|draft| {
        draft.set(&JsonPointer::parse("/values/0").unwrap(), json!("changed"));
    })
    .unwrap();

    let after = array_items(&form, array);
    assert_eq!(after[0].1, before[0].1);
    assert_eq!(after[1].1, before[1].1);
    assert_eq!(
        form.form_data(),
        &json!({ "values": ["changed", "second"] })
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn host_append_preserves_existing_array_item_identities() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": { "type": "array", "items": { "type": "string" } }
        }
    }))
    .unwrap();
    let mut form = definition
        .create_form(json!({ "values": ["first", "second"] }))
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let before = array_items(&form, array);

    let transition = form
        .transact(|draft| {
            draft.append_item(&JsonPointer::parse("/values").unwrap(), json!("third"));
        })
        .expect("the host append should commit atomically");

    let after = array_items(&form, array);
    assert_eq!(
        form.form_data(),
        &json!({ "values": ["first", "second", "third"] })
    );
    assert_eq!(after[0], before[0]);
    assert_eq!(after[1], before[1]);
    assert_ne!(after[2].1, before[0].1);
    assert_ne!(after[2].1, before[1].1);
    assert!(transition.changed().any(|identity| identity == array));
    assert!(transition.changed().any(|identity| identity == after[2].0));
    assert_eq!(transition.removed().count(), 0);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn host_structural_commands_preserve_surviving_logical_items() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": { "type": "array", "items": { "type": "number" } }
        }
    }))
    .unwrap();
    let mut form = definition
        .form(json!({ "values": [1, 1] }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::Immediate,
        ))
        .build()
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let initial = array_items(&form, array);
    let logical_item = initial[1];
    form.user()
        .input_text(logical_item.0, "not a number")
        .unwrap();
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "review",
            JsonPointer::parse("/values/1").unwrap(),
            json!({}),
        )],
    ))
    .unwrap();

    let moved = form
        .transact(|draft| {
            draft.move_item_up(&JsonPointer::parse("/values").unwrap(), logical_item.1);
        })
        .expect("the host move should commit atomically");

    assert_eq!(array_items(&form, array), vec![logical_item, initial[0]]);
    let logical_node = form.node(logical_item.0).unwrap();
    assert_eq!(
        logical_node.binding().unwrap().pointer().as_str(),
        "/values/0"
    );
    assert_eq!(logical_node.edit_buffer(), Some("not a number"));
    assert_eq!(
        logical_node
            .external_findings()
            .next()
            .unwrap()
            .1
            .instance_location()
            .as_str(),
        "/values/0"
    );
    assert!(moved.changed().any(|identity| identity == logical_item.0));
    assert!(moved.changed().any(|identity| identity == initial[0].0));

    form.transact(|draft| {
        draft.insert_item_before(
            &JsonPointer::parse("/values").unwrap(),
            initial[0].1,
            json!(2),
        );
    })
    .expect("the host insert should commit atomically");
    let inserted = array_items(&form, array);
    assert_eq!(inserted[0], logical_item);
    assert_eq!(inserted[2], initial[0]);
    assert_eq!(form.form_data(), &json!({ "values": [1, 2, 1] }));

    let inserted_item = inserted[1];
    let removed = form
        .transact(|draft| {
            draft.remove_item(&JsonPointer::parse("/values").unwrap(), inserted_item.1);
            draft.move_item_down(&JsonPointer::parse("/values").unwrap(), logical_item.1);
        })
        .expect("related host structural commands should commit atomically");
    assert_eq!(array_items(&form, array), initial);
    assert!(form.node(inserted_item.0).is_none());
    assert!(
        removed
            .removed()
            .any(|identity| identity == inserted_item.0)
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn reinitialize_allocates_fresh_identity_for_semantically_equal_array_data() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": { "type": "array", "items": { "type": "string" } }
        }
    }))
    .unwrap();
    let data = json!({ "values": ["same", "same"] });
    let mut form = definition.create_form(data.clone()).unwrap();
    let array = node_with_binding(&form, "/values");
    let before = array_items(&form, array);

    let transition = form
        .reinitialize(data)
        .expect("equal data should still start a fresh repeated topology");

    let after = array_items(&form, array);
    assert_eq!(after.len(), before.len());
    assert!(
        after
            .iter()
            .all(|after| before.iter().all(|before| after.1 != before.1))
    );
    for item in before {
        assert!(form.node(item.0).is_none());
        assert!(transition.removed().any(|identity| identity == item.0));
    }
    for item in after {
        assert!(transition.changed().any(|identity| identity == item.0));
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn changed_whole_array_replacement_allocates_all_fresh_identities() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": { "type": "array", "items": { "type": "string" } }
        }
    }))
    .unwrap();
    let mut form = definition
        .create_form(json!({ "values": ["same", "old"] }))
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let before = array_items(&form, array);

    let transition = form
        .transact(|draft| {
            draft.set(
                &JsonPointer::parse("/values").unwrap(),
                json!(["same", "new", "same"]),
            );
        })
        .expect("the authoritative whole-array replacement should commit");

    let after = array_items(&form, array);
    assert!(
        after
            .iter()
            .all(|after| before.iter().all(|before| after.1 != before.1))
    );
    assert_eq!(
        form.form_data(),
        &json!({ "values": ["same", "new", "same"] })
    );
    for item in before {
        assert!(form.node(item.0).is_none());
        assert!(transition.removed().any(|identity| identity == item.0));
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn equal_whole_array_write_before_append_preserves_existing_identity() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": { "type": "array", "items": { "type": "string" } }
        }
    }))
    .unwrap();
    let mut form = definition
        .create_form(json!({ "values": ["first"] }))
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let existing = array_items(&form, array)[0];

    form.transact(|draft| {
        draft.set(&JsonPointer::parse("/values").unwrap(), json!(["first"]));
        draft.append_item(&JsonPointer::parse("/values").unwrap(), json!("second"));
    })
    .expect("an equal authoritative write and explicit append should commit");

    let after = array_items(&form, array);
    assert_eq!(after[0], existing);
    assert_ne!(after[1].1, existing.1);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn topology_only_reset_rebases_preserved_findings_to_baseline_identities() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": { "type": "array", "items": { "type": "number" } }
        }
    }))
    .unwrap();
    let mut form = definition
        .form(json!({ "values": [1, 1] }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::Immediate,
        ))
        .build()
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let baseline = array_items(&form, array);
    let logical_item = baseline[1];
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "review",
            JsonPointer::parse("/values/1").unwrap(),
            json!({}),
        )],
    ))
    .unwrap();
    form.user().move_item_up(array, logical_item.1).unwrap();
    assert_eq!(
        form.node(logical_item.0)
            .unwrap()
            .external_findings()
            .next()
            .unwrap()
            .1
            .instance_location()
            .as_str(),
        "/values/0"
    );

    let reset = form.reset();

    assert_eq!(array_items(&form, array), baseline);
    assert!(reset.changed().any(|changed| changed == logical_item.0));
    assert_eq!(
        form.node(baseline[0].0)
            .unwrap()
            .external_findings()
            .count(),
        0
    );
    assert_eq!(
        form.node(logical_item.0)
            .unwrap()
            .external_findings()
            .next()
            .unwrap()
            .1
            .instance_location()
            .as_str(),
        "/values/1"
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn reset_preserves_current_path_finding_when_replacing_post_baseline_identity() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": {
                "type": "array",
                "items": { "type": "number", "default": 0 }
            }
        }
    }))
    .unwrap();
    let mut form = definition
        .form(json!({ "values": [0, 0] }))
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::Immediate,
        ))
        .build()
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let baseline = array_items(&form, array);
    form.user().remove_item(array, baseline[0].1).unwrap();
    form.user().append_item(array).unwrap();
    form.apply_external_findings(ExternalFindingBatch::new(
        "server",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "review",
            JsonPointer::parse("/values/1").unwrap(),
            json!({}),
        )],
    ))
    .unwrap();

    form.reset();

    assert_eq!(array_items(&form, array), baseline);
    assert_eq!(
        form.node(baseline[1].0)
            .unwrap()
            .external_findings()
            .next()
            .unwrap()
            .1
            .instance_location()
            .as_str(),
        "/values/1"
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn mixed_host_moves_and_writes_clear_edit_state_by_logical_identity() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": { "type": "array", "items": { "type": "number" } }
        }
    }))
    .unwrap();
    let pointer = JsonPointer::parse("/values").unwrap();

    let mut move_then_set = definition.create_form(json!({ "values": [1, 1] })).unwrap();
    let array = node_with_binding(&move_then_set, "/values");
    let items = array_items(&move_then_set, array);
    move_then_set
        .user()
        .input_text(items[1].0, "not a number")
        .unwrap();
    move_then_set
        .transact(|draft| {
            draft.move_item_up(&pointer, items[1].1);
            draft.set(&JsonPointer::parse("/values/0").unwrap(), json!(2));
        })
        .unwrap();
    assert_eq!(move_then_set.node(items[1].0).unwrap().edit_buffer(), None);
    assert_eq!(
        move_then_set.node(items[1].0).unwrap().parse_blocker(),
        None
    );

    let mut set_then_move = definition.create_form(json!({ "values": [1, 1] })).unwrap();
    let array = node_with_binding(&set_then_move, "/values");
    let items = array_items(&set_then_move, array);
    set_then_move
        .user()
        .input_text(items[0].0, "not a number")
        .unwrap();
    set_then_move
        .transact(|draft| {
            draft.set(&JsonPointer::parse("/values/0").unwrap(), json!(2));
            draft.move_item_down(&pointer, items[0].1);
        })
        .unwrap();
    assert_eq!(set_then_move.node(items[0].0).unwrap().edit_buffer(), None);
    assert_eq!(
        set_then_move.node(items[0].0).unwrap().parse_blocker(),
        None
    );
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn removing_last_item_finding_drops_empty_external_batch() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["values"],
        "properties": {
            "values": { "type": "array", "items": { "type": "number" } }
        }
    }))
    .unwrap();
    let mut form = definition
        .form(json!({ "values": [1, 1] }))
        .external_finding_limits(ExternalFindingLimits::new(
            NonZeroUsize::new(2).unwrap(),
            NonZeroUsize::new(15).unwrap(),
        ))
        .build()
        .unwrap();
    let array = node_with_binding(&form, "/values");
    let removed = array_items(&form, array)[0];
    form.apply_external_findings(ExternalFindingBatch::new(
        "old",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "x",
            JsonPointer::parse("/values/0").unwrap(),
            json!({}),
        )],
    ))
    .unwrap();

    form.transact(|draft| {
        draft.remove_item(&JsonPointer::parse("/values").unwrap(), removed.1);
        draft.append_item(&JsonPointer::parse("/values").unwrap(), json!(1));
    })
    .unwrap();

    form.apply_external_findings(ExternalFindingBatch::new(
        "n",
        form.view().data_revision(),
        [ExternalFinding::blocking(
            "x",
            JsonPointer::parse("/values/0").unwrap(),
            json!({}),
        )],
    ))
    .expect("an emptied prior source must not consume the finding byte budget");
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
        pending.extend(node.children());
    }
    panic!("the bound node should exist")
}

fn array_items(form: &Form, array: InstanceIdentity) -> Vec<(InstanceIdentity, ItemIdentity)> {
    form.node(array)
        .expect("the array node should exist")
        .children()
        .map(|identity| {
            let item = form
                .node(identity)
                .and_then(|node| node.binding())
                .and_then(|binding| binding.item())
                .expect("an array item should have opaque identity");
            (identity, item)
        })
        .collect()
}
