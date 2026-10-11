//! Public-facade tests for a form made read-only as a whole.
//!
//! A read-only form refuses every user operation while host operations keep working, and lifting
//! it restores what the data schema's own annotations allow.

use schemaform::{
    Form, FormDefinition, InstanceIdentity, SubmissionOutcome,
    form::{AllowedOperations, UserOperationError},
};
use serde_json::{Value, json};

fn definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string" },
            "level": { "type": ["integer", "null"] },
            "tags": { "type": "array", "items": { "type": "string" } },
            "colours": {
                "type": "array",
                "uniqueItems": true,
                "items": { "enum": ["red", "green"] }
            },
            "reference": { "type": "string", "readOnly": true }
        }
    }))
    .expect("the data schema should compile")
}

fn form_data() -> Value {
    json!({
        "name": "Ada",
        "level": 1,
        "tags": ["a", "b"],
        "colours": ["red"],
        "reference": "ref_42"
    })
}

#[test]
fn a_read_only_form_refuses_every_user_operation() {
    let mut form = definition()
        .form(form_data())
        .read_only()
        .build()
        .expect("the form should be created");
    assert!(form.is_read_only());
    let name = node_with_binding(&form, "/name");
    let level = node_with_binding(&form, "/level");
    let tags = node_with_binding(&form, "/tags");
    let colours = node_with_binding(&form, "/colours");
    let first_tag = form
        .node(tags)
        .and_then(|node| node.children().next())
        .and_then(|item| form.node(item).and_then(|node| node.item_identity()))
        .expect("the first tag should have an item identity");

    for identity in all_nodes(&form) {
        let node = form.node(identity).expect("a listed node exists");
        assert!(node.is_read_only());
        assert_eq!(node.allowed_operations(), AllowedOperations::default());
    }

    let mut user = form.user();
    let refused = [
        user.input_text(name, "Grace"),
        user.blur(name),
        user.set_value(level, json!(2)),
        user.set_null(level),
        user.remove_value(level),
        user.replace_value(level, json!(3)),
        user.append_item(tags),
        user.insert_item_before(tags, first_tag),
        user.remove_item(tags, first_tag),
        user.move_item_down(tags, first_tag),
        user.toggle_choice(colours, json!("green")),
    ];
    for (index, result) in refused.into_iter().enumerate() {
        assert_eq!(
            result.err(),
            Some(UserOperationError::OperationNotAllowed),
            "operation {index} should be refused"
        );
    }
    assert_eq!(form.form_data(), &form_data());
}

#[test]
fn host_operations_keep_working_on_a_read_only_form() {
    let mut form = definition()
        .form(form_data())
        .read_only()
        .build()
        .expect("the form should be created");

    form.transact(|transaction| {
        transaction.set(
            &schemaform::JsonPointer::parse("/name").expect("the pointer is valid"),
            json!("Grace"),
        );
    })
    .expect("a host transaction should commit on a read-only form");
    assert_eq!(form.form_data()["name"], json!("Grace"));

    assert!(matches!(
        form.prepare_submission().into_parts().1,
        SubmissionOutcome::Ready(_)
    ));
}

#[test]
fn toggling_read_only_reports_the_nodes_it_changes_and_restores_schema_permissions() {
    let mut form = definition()
        .create_form(form_data())
        .expect("the form should be created");
    let name = node_with_binding(&form, "/name");
    let reference = node_with_binding(&form, "/reference");
    assert!(!form.is_read_only());
    assert!(!form.node(name).expect("name exists").is_read_only());
    assert!(
        form.node(reference)
            .expect("reference exists")
            .is_read_only()
    );

    let transition = form.set_read_only(true);
    assert!(transition.changed().any(|node| node == name));
    // The annotated node was read-only already: nothing observable about it changed.
    assert!(!transition.changed().any(|node| node == reference));
    assert!(form.node(name).expect("name exists").is_read_only());
    assert!(form.set_read_only(true).changed().next().is_none());

    form.set_read_only(false);
    assert!(!form.node(name).expect("name exists").is_read_only());
    assert!(
        form.node(reference)
            .expect("reference exists")
            .is_read_only()
    );
    form.user()
        .input_text(name, "Grace")
        .expect("lifting read-only restores editing");
}

fn all_nodes(form: &Form) -> Vec<InstanceIdentity> {
    let mut pending = vec![form.view().root()];
    let mut nodes = Vec::new();
    while let Some(identity) = pending.pop() {
        nodes.push(identity);
        pending.extend(
            form.node(identity)
                .expect("a listed node exists")
                .children(),
        );
    }
    nodes
}

fn node_with_binding(form: &Form, binding: &str) -> InstanceIdentity {
    all_nodes(form)
        .into_iter()
        .find(|identity| {
            form.node(*identity)
                .and_then(|node| node.binding().map(|binding| binding.pointer().clone()))
                .is_some_and(|pointer| pointer.as_str() == binding)
        })
        .expect("the bound node should exist")
}
