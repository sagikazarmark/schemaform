use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

use crate::components::{StatusLine, schemaform_daisyui};

/// Generated presentation needs only a Draft 2020-12 data schema. This one
/// exercises text, integer, boolean, choice, multiple choice, nullable,
/// constant, read-only, and write-only controls as well as validation and
/// submission. The two choices show both spellings side by side: a plain
/// `enum` whose options read as their values, and a constant choice (`oneOf`
/// of titled constants) whose options read as their titles in authored order.
/// The multiple choice is a `uniqueItems` array of titled constants: one
/// checkbox per option, members kept in option order, `minItems` left as a
/// finding rather than a disabled option. Five strings carry a `format` the
/// browser has a widget for, one per widget: `email`, `uri`, `date`,
/// `date-time`, and `time`. The controls are the built-in renderer's; only the
/// shell and the finding summary come from the demo's daisyUI component.
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
            "priority": "normal",
            "channels": ["email"],
            "nickname": null,
            "email": "ada@example.test",
            "homepage": "https://example.test/ada",
            "born_on": "1815-12-10",
            "last_seen": "2024-05-06T09:30",
            "daily_digest_at": "08:00",
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
                    if let Some(access_token) = displayed
                        .as_object_mut()
                        .and_then(|object| object.get_mut("access_token"))
                    {
                        *access_token = serde_json::Value::String("[redacted]".to_owned());
                    }
                    submitted.set(
                        serde_json::to_string_pretty(&displayed)
                            .expect("form data should serialize"),
                    );
                },
                on_error: move |error| crate::examples::report_form_error(&error),
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
        "required": ["name", "age", "active", "plan", "priority", "channels", "account_type"],
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
            "priority": {
                "title": "Support priority",
                "oneOf": [
                    {
                        "const": "low",
                        "title": "Low priority",
                        "description": "Answered within the week."
                    },
                    {
                        "const": "normal",
                        "title": "Normal priority",
                        "description": "Answered within a business day."
                    },
                    {
                        "const": "high",
                        "title": "High priority",
                        "description": "Escalated within the hour."
                    }
                ]
            },
            "channels": {
                "type": "array",
                "title": "Notification channels",
                "description": "Pick at least one.",
                "uniqueItems": true,
                "minItems": 1,
                "items": {
                    "oneOf": [
                        { "const": "email", "title": "Email" },
                        { "const": "sms", "title": "Text message" },
                        { "const": "push", "title": "Push notification" }
                    ]
                }
            },
            "nickname": {
                "type": ["string", "null"],
                "title": "Nickname"
            },
            "email": {
                "type": "string",
                "title": "Email",
                "description": "Rendered as the browser's email widget.",
                "format": "email"
            },
            "homepage": {
                "type": "string",
                "title": "Homepage",
                "description": "Rendered as the browser's URL widget.",
                "format": "uri"
            },
            "born_on": {
                "type": "string",
                "title": "Born on",
                "description": "Rendered as the browser's date picker.",
                "format": "date"
            },
            "last_seen": {
                "type": "string",
                "title": "Last seen",
                "description": "Rendered as the browser's local date-time picker; the value carries no zone offset.",
                "format": "date-time"
            },
            "daily_digest_at": {
                "type": "string",
                "title": "Daily digest at",
                "description": "Rendered as the browser's time picker.",
                "format": "time"
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
