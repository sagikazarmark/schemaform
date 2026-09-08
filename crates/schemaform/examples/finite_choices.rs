use schemaform::{Form, FormDefinition, InstanceIdentity};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "priority": {
                "title": "Priority",
                "oneOf": [
                    { "const": "low", "title": "Low", "description": "When there is time." },
                    { "const": "high", "title": "High" }
                ]
            },
            "channels": {
                "title": "Channels",
                "type": "array",
                "uniqueItems": true,
                "items": {
                    "oneOf": [
                        { "const": "email", "title": "Email" },
                        { "const": "sms", "title": "SMS" },
                        { "const": "push", "title": "Push" }
                    ]
                }
            }
        }
    }))?;
    let mut form = definition.create_form(json!({ "priority": "low" }))?;

    // A constant choice is the scalar choice control `enum` produces: options
    // in authored order, labeled by their branch `title`, selected by value.
    let priority = control_at(&form, "/priority");
    let node = form.node(priority).expect("the priority control exists");
    assert!(node.definition().is_choice_selectable());
    assert_eq!(
        node.definition()
            .choice_options()
            .map(|option| (option.label(), option.description()))
            .collect::<Vec<_>>(),
        [("Low", Some("When there is time.")), ("High", None)]
    );
    assert_eq!(node.display_text().as_deref(), Some("Low"));
    form.user().set_value(priority, json!("high"))?;
    assert_eq!(form.form_data()["priority"], json!("high"));

    // A multiple choice stays an array node — items, identities, collection
    // operations — and additionally offers a toggle per option. It is a leaf
    // control, so the first toggle creates the absent array as typing into an
    // absent string creates the string. Members keep option order, not check
    // order; toggling a held value removes it.
    let channels = control_at(&form, "/channels");
    let node = form.node(channels).expect("the channels array exists");
    assert!(node.definition().is_multiple_choice());
    assert_eq!(
        node.definition()
            .choice_options()
            .map(|option| option.label())
            .collect::<Vec<_>>(),
        ["Email", "SMS", "Push"]
    );
    assert!(form.form_data().get("channels").is_none());
    form.user().toggle_choice(channels, json!("push"))?;
    assert_eq!(form.form_data()["channels"], json!(["push"]));
    form.user().toggle_choice(channels, json!("email"))?;
    assert_eq!(form.form_data()["channels"], json!(["email", "push"]));
    form.user().toggle_choice(channels, json!("push"))?;
    assert_eq!(form.form_data()["channels"], json!(["email"]));

    println!("{}", form.form_data());
    Ok(())
}

/// The form-tree node bound to `pointer`.
fn control_at(form: &Form, pointer: &str) -> InstanceIdentity {
    let mut pending = vec![form.view().root()];
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).expect("a listed node exists");
        if node
            .binding()
            .is_some_and(|binding| binding.pointer().as_str() == pointer)
        {
            return identity;
        }
        pending.extend(node.children());
    }
    panic!("no control is bound to {pointer}")
}
