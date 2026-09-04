use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

use crate::components::{StatusLine, schemaform_daisyui};

/// The same generated form as the built-in page, bound through the daisyUI
/// control registry: every string, number, and integer control is a daisyUI
/// `Field` with an `Input`, label, help, errors, and presence affordances, while
/// the boolean, choice, and constant controls still come from the built-in
/// renderer.
#[component]
pub fn DaisyuiTextControlsExample() -> Element {
    let definition = use_hook(definition);
    let form = use_form(
        definition,
        json!({
            "name": "Ada",
            "age": 36,
            "price": 19.5,
            "active": true,
            "plan": "team",
            "nickname": null,
            "account_type": "standard",
            "customer_id": "cus_1843",
            "access_token": "not-rendered"
        }),
    )
    .expect("the daisyUI example form should be created");
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .controls(schemaform_daisyui::controls())
            .build()
            .bind(&bound_form)
            .expect("the daisyUI registry should bind every control")
    });
    let mut submitted = use_signal(String::new);
    let reset_form = form.clone();

    rsx! {
        div { class: "space-y-4",
            SchemaForm {
                form: bound,
                on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                    let mut displayed = snapshot.form_data().clone();
                    if let Some(access_token) = displayed.get_mut("access_token") {
                        *access_token = serde_json::Value::String("[redacted]".to_owned());
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
        "required": ["name", "age", "price", "active", "plan", "account_type"],
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
            "price": {
                "type": "number",
                "title": "Monthly price",
                "description": "In euros, decimals allowed.",
                "minimum": 0
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
    .expect("the daisyUI example data schema should compile")
}

#[cfg(test)]
mod tests {
    #[test]
    fn example_data_schema_compiles() {
        super::definition();
    }
}
