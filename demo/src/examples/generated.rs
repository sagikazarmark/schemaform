use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

use crate::components::{StatusLine, schemaform_daisyui};

/// Generated presentation needs only a Draft 2020-12 data schema. This one
/// exercises text, integer, boolean, choice, nullable, constant, read-only, and
/// write-only controls as well as validation and submission. The controls are
/// the built-in renderer's; only the shell and the finding summary come from
/// the demo's daisyUI component.
#[component]
pub fn GeneratedControlsExample() -> Element {
    let definition = use_hook(definition);
    let form = use_form(
        definition,
        json!({
            "name": "Ada",
            "age": 36,
            "active": true,
            "plan": "team",
            "nickname": null,
            "account_type": "standard",
            "customer_id": "cus_1843",
            "access_token": "not-rendered"
        }),
    )
    .expect("the generated example form should be created");
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .structure(schemaform_daisyui::structure())
            .summary_presenter(schemaform_daisyui::findings())
            .build()
            .bind(&bound_form)
            .expect("built-in controls should bind")
    });
    let mut submitted = use_signal(String::new);
    let reset_form = form.clone();

    rsx! {
        div { class: "space-y-4",
            SchemaForm {
                form: bound,
                on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                    let mut displayed = snapshot.form_data().clone();
                    if let Some(object) = displayed.as_object_mut() {
                        if let Some(access_token) = object.get_mut("access_token") {
                            *access_token = serde_json::Value::String("[redacted]".to_owned());
                        }
                    }
                    submitted.set(
                        serde_json::to_string_pretty(&displayed)
                            .expect("form data should serialize"),
                    );
                },
                on_error: move |error| eprintln!("form operation failed: {error}"),
            }
            button {
                class: "btn btn-sm btn-ghost",
                r#type: "button",
                onclick: move |_| {
                    if reset_form.reset().is_ok() {
                        submitted.set(String::new());
                    }
                },
                "Reset to baseline"
            }
            StatusLine { status: submitted.read().clone() }
        }
    }
}

fn definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "age", "active", "plan", "account_type"],
        "properties": {
            "name": {
                "type": "string",
                "title": "Display name",
                "description": "At least three characters.",
                "minLength": 3
            },
            "age": {
                "type": "integer",
                "title": "Age",
                "minimum": 18
            },
            "active": {
                "type": "boolean",
                "title": "Account is active"
            },
            "plan": {
                "title": "Plan",
                "enum": ["starter", "team", "enterprise"]
            },
            "nickname": {
                "type": ["string", "null"],
                "title": "Nickname"
            },
            "account_type": {
                "title": "Account type",
                "const": "standard"
            },
            "customer_id": {
                "type": "string",
                "title": "Customer ID",
                "readOnly": true
            },
            "access_token": {
                "type": "string",
                "title": "Access token",
                "writeOnly": true
            }
        }
    }))
    .expect("the generated example schema should compile")
}

#[cfg(test)]
mod tests {
    #[test]
    fn example_schema_compiles() {
        super::definition();
    }
}
