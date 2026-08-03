use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{SchemaForm, RenderConfiguration, use_form};
use serde_json::json;

use crate::components::StatusLine;

/// The basic path compiles one trusted data schema, creates form state from
/// canonical JSON data, and binds the built-in Dioxus renderers.
#[component]
pub fn MinimalExample() -> Element {
    let definition = use_hook(definition);
    let form = use_form(definition, json!({ "name": "Ada" })).expect("the form should be created");
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::default()
            .bind(&bound_form)
            .expect("the built-in string control should bind")
    });
    let mut greeting = use_signal(String::new);

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                let name = snapshot.form_data()["name"].as_str().unwrap_or_default();
                greeting.set(format!("Hello, {name}!"));
            },
            on_error: move |error| eprintln!("form operation failed: {error}"),
        }
        StatusLine { status: greeting.read().clone() }
    }
}

fn definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "title": "Your name" }
        }
    }))
    .expect("the data schema should compile")
}

#[cfg(test)]
mod tests {
    #[test]
    fn example_schema_compiles() {
        super::definition();
    }
}
