use schemaform::FormDefinition;
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string", "default": "Ada" },
            "quantity": { "type": "integer", "minimum": 1, "default": 1 },
            "shipping": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "country": { "type": "string", "default": "GB" }
                }
            }
        }
    }))?;

    // `create_form` leaves an absent member absent, whatever its `default` says.
    let form = definition.create_form(json!({ "quantity": 4 }))?;
    assert_eq!(form.form_data(), &json!({ "quantity": 4 }));

    // Seeding fills the absent scalars. The host's `quantity` wins, and the
    // absent `shipping` object is not invented although `country` has a default.
    let form = definition.create_form_with_defaults(json!({ "quantity": 4 }))?;
    assert_eq!(form.form_data(), &json!({ "name": "Ada", "quantity": 4 }));

    // The seeded data is the baseline: nothing is dirty until the user edits.
    let root = form.node(form.view().root()).expect("the root exists");
    for child in root.children() {
        let control = form.node(child).expect("a listed node exists");
        assert!(!control.is_dirty(), "a seeded control is not dirty");
    }

    // The same switch on the builder, for combining with limits or visibility.
    let form = definition.form(json!({})).seed_defaults().build()?;
    assert_eq!(form.form_data(), &json!({ "name": "Ada", "quantity": 1 }));

    println!("{}", form.form_data());
    Ok(())
}
