use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
fn main() {
    dioxus_web::launch::launch(App, Vec::new(), Vec::new());
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn main() {}

#[component]
fn App() -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "required": ["name"],
            "properties": {
                "name": { "type": "string", "title": "Your name" }
            }
        }))
        .expect("the trusted data schema should compile")
    });
    let form = use_form(definition, json!({ "name": "Ada" })).expect("the form should be created");
    let form_to_bind = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::default()
            .bind(&form_to_bind)
            .expect("the built-in renderer should bind")
    });
    let mut submitted = use_signal(String::new);

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                submitted.set(snapshot.form_data().to_string())
            },
            on_error: move |error| eprintln!("form operation failed: {error}"),
        }
        output { "{submitted}" }
    }
}
