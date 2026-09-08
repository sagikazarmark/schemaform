use schemaform::{
    Form, FormDefinition, InstanceIdentity, SubmissionOutcome,
    definition::CompilationProfile,
    form::ValidationOutcomeView,
    json::{FormDataLimits, parse_ui_schema_v1},
};
use serde_json::json;

#[test]
fn seeding_fills_absent_scalars_from_their_default_and_leaves_present_members_alone() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string", "default": "Ada" },
            "age": { "type": "integer", "default": 36 },
            "active": { "type": "boolean", "default": true },
            "plan": { "enum": ["starter", "team"], "default": "team" },
            "priority": {
                "oneOf": [
                    { "const": "low", "title": "Low" },
                    { "const": "high", "title": "High" }
                ],
                "default": "high"
            },
            "kind": { "const": "person", "default": "person" },
            "nickname": { "type": ["string", "null"], "default": "ada" },
            "note": { "type": "string" }
        }
    }))
    .expect("the seeding data schema should compile");

    let form = definition
        .create_form_with_defaults(json!({ "age": 18, "nickname": null }))
        .expect("the seeded form should be created");

    assert_eq!(
        form.form_data(),
        &json!({
            "name": "Ada",
            "age": 18,
            "active": true,
            "plan": "team",
            "priority": "high",
            "kind": "person",
            "nickname": null
        })
    );
}

#[test]
fn seeding_fills_scalars_inside_present_objects_and_leaves_absent_objects_absent() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "address": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "street": { "type": "string", "default": "Main St" },
                    "country": { "type": "string", "default": "GB" }
                }
            },
            "billing": {
                "type": "object",
                "additionalProperties": false,
                "default": { "country": "US" },
                "properties": {
                    "country": { "type": "string", "default": "GB" }
                }
            }
        }
    }))
    .expect("the nested seeding data schema should compile");

    let form = definition
        .create_form_with_defaults(json!({ "address": { "street": "Baker St" } }))
        .expect("the seeded nested form should be created");

    assert_eq!(
        form.form_data(),
        &json!({ "address": { "street": "Baker St", "country": "GB" } }),
        "the present object is completed, the absent object with its own default stays absent"
    );
}

#[test]
fn seeding_applies_item_defaults_only_to_array_items_that_exist() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "lines": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "sku": { "type": "string" },
                        "quantity": { "type": "integer", "default": 1 }
                    }
                }
            },
            "tags": {
                "type": "array",
                "default": ["draft"],
                "items": { "type": "string", "default": "tag" }
            }
        }
    }))
    .expect("the array seeding data schema should compile");

    let form = definition
        .create_form_with_defaults(json!({
            "lines": [{ "sku": "a" }, { "sku": "b", "quantity": 4 }]
        }))
        .expect("the seeded array form should be created");

    assert_eq!(
        form.form_data(),
        &json!({
            "lines": [
                { "sku": "a", "quantity": 1 },
                { "sku": "b", "quantity": 4 }
            ]
        }),
        "existing items are completed, the absent array is not created, no item is invented"
    );

    let empty = definition
        .create_form_with_defaults(json!({ "lines": [], "tags": [] }))
        .expect("the empty-array form should be created");
    assert_eq!(empty.form_data(), &json!({ "lines": [], "tags": [] }));
}

#[test]
fn a_default_violating_its_own_schema_is_seeded_and_reported_as_a_finding() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "quantity": { "type": "integer", "minimum": 1, "default": 0 },
            "code": { "type": "string", "default": 7 }
        }
    }))
    .expect("a schema whose defaults violate it should still compile");

    let mut form = definition
        .create_form_with_defaults(json!({}))
        .expect("the seeded form should be created");

    assert_eq!(form.form_data(), &json!({ "quantity": 0, "code": 7 }));
    let view = form.view();
    let ValidationOutcomeView::Invalid { findings, .. } = view.validation_outcome() else {
        panic!("seeded defaults that violate their schema should be reported");
    };
    let mut reported = findings
        .iter()
        .map(|finding| (finding.instance_location().as_str(), finding.code()))
        .collect::<Vec<_>>();
    reported.sort_unstable();
    assert_eq!(reported, [("/code", "type"), ("/quantity", "minimum")]);
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));
}

#[test]
fn a_null_default_is_seeded_as_null_without_second_guessing() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "nickname": { "type": ["string", "null"], "default": null },
            "code": { "type": "string", "default": null }
        }
    }))
    .expect("null defaults are annotations and should compile");

    let form = definition
        .create_form_with_defaults(json!({}))
        .expect("the seeded form should be created");

    assert_eq!(form.form_data(), &json!({ "nickname": null, "code": null }));
    let view = form.view();
    let ValidationOutcomeView::Invalid { findings, .. } = view.validation_outcome() else {
        panic!("the null default on a non-nullable string should be reported");
    };
    assert_eq!(
        findings
            .iter()
            .map(|finding| (finding.instance_location().as_str(), finding.code()))
            .collect::<Vec<_>>(),
        [("/code", "type")],
        "the nullable scalar accepts its null default; only the other one is a finding"
    );
}

#[test]
fn seeded_data_is_the_baseline_that_reset_restores_and_dirty_state_compares_against() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "quantity"],
        "properties": {
            "name": { "type": "string", "default": "Ada" },
            "quantity": { "type": "integer", "minimum": 1, "default": 3 }
        }
    }))
    .expect("the baseline data schema should compile");

    let mut form = definition
        .create_form_with_defaults(json!({ "name": "Lin" }))
        .expect("the seeded form should be created");
    let equivalent = definition
        .create_form(json!({ "name": "Lin", "quantity": 3 }))
        .expect("the equivalently seeded form should be created");
    let name = node_with_binding(&form, "/name");
    let quantity = node_with_binding(&form, "/quantity");

    // Revisions are form-scoped and compare only within one form, so "starts
    // where an equivalently seeded `create_form` would" is observed through
    // what a fresh form exposes: the same data and a settled lifecycle.
    assert_eq!(form.form_data(), equivalent.form_data());
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
    assert!(!form.view().submission_attempted());
    for identity in [name, quantity] {
        let node = form.node(identity).expect("the control should exist");
        assert_eq!(node.edit_buffer(), None);
        assert!(!node.is_touched(), "seeding must not touch {identity:?}");
        assert!(!node.is_dirty(), "seeding must not dirty {identity:?}");
    }

    form.user()
        .input_text(quantity, "9")
        .expect("the seeded quantity should accept an edit");
    assert!(
        form.node(quantity)
            .expect("the quantity should remain present")
            .is_dirty()
    );
    form.reset();
    assert_eq!(form.form_data(), &json!({ "name": "Lin", "quantity": 3 }));
    assert!(
        !form
            .node(quantity)
            .expect("the quantity should remain present")
            .is_dirty()
    );
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

#[test]
fn seeding_skips_a_scalar_whose_applicable_schemas_disagree_on_its_default() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "agreed": {
                "allOf": [
                    { "type": "string", "default": "same" },
                    { "default": "same" }
                ]
            },
            "disputed": {
                "allOf": [
                    { "type": "string", "default": "one" },
                    { "default": "other" }
                ]
            }
        }
    }))
    .expect("conflicting defaults are annotations and should compile");

    let form = definition
        .create_form_with_defaults(json!({}))
        .expect("the seeded form should be created");

    assert_eq!(form.form_data(), &json!({ "agreed": "same" }));
}

#[test]
fn create_form_and_the_plain_builder_never_seed_defaults() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string", "default": "Ada" }
        }
    }))
    .expect("the data schema should compile");

    let plain = definition
        .create_form(json!({}))
        .expect("the unseeded form should be created");
    let built = definition
        .form(json!({}))
        .build()
        .expect("the unseeded built form should be created");
    let seeded = definition
        .form(json!({}))
        .seed_defaults()
        .limits(FormDataLimits::default())
        .build()
        .expect("the seeded built form should be created");

    assert_eq!(plain.form_data(), &json!({}));
    assert_eq!(built.form_data(), &json!({}));
    assert_eq!(seeded.form_data(), &json!({ "name": "Ada" }));
}

#[test]
fn seeding_follows_the_data_schema_not_the_authored_ui_schema() {
    let ui_schema = parse_ui_schema_v1(
        br#"{
            "version": 1,
            "root": {
                "type": "control",
                "value": { "binding": { "origin": "root", "pointer": "/shown" } }
            }
        }"#,
        &CompilationProfile::default(),
    )
    .expect("the single-control UI schema should parse");
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "shown": { "type": "string", "default": "visible" },
            "omitted": { "type": "string", "default": "still seeded" }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored definition should compile");

    let form = definition
        .create_form_with_defaults(json!({}))
        .expect("the seeded authored form should be created");

    assert_eq!(
        form.form_data(),
        &json!({ "shown": "visible", "omitted": "still seeded" }),
        "a default is a data-schema annotation; leaving a control out of the layout does not unseed it"
    );
}
