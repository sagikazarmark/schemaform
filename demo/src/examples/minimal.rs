use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

use crate::components::{StatusLine, schemaform_daisyui};

/// The basic path compiles one trusted data schema, creates form state from
/// canonical JSON data, and binds the built-in control renderers. The form
/// shell and the finding summary are the demo's daisyUI component's, so the
/// submit button and the summary alert match the rest of the gallery.
#[component]
pub fn MinimalExample() -> Element {
    let definition = use_hook(definition);
    let form = use_form(definition, json!({ "name": "Ada" })).expect("the form should be created");
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .structure(schemaform_daisyui::structure())
            .summary_presenter(schemaform_daisyui::findings())
            .build()
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
            on_error: move |error| crate::examples::report_form_error(&error),
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
