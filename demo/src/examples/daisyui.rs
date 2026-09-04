use dioxus::prelude::*;
use schemaform::{CompilationProfile, FormDefinition, json::parse_ui_schema_v1};
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

use crate::components::{StatusLine, schemaform_daisyui};

/// The authored presentation: every control in data-schema order, except that the billing
/// cycle asks for the `daisyui:radio` widget and the region for `daisyui:select`. Every other
/// choice is the default native select.
const UI_SCHEMA: &str = r#"{
  "version": 1,
  "root": {
    "type": "stack",
    "value": {
      "children": [
        {
          "type": "auto",
          "value": {
            "binding": { "origin": "root", "pointer": "" },
            "properties": { "exclude": ["billing_cycle", "region"] }
          }
        },
        {
          "type": "control",
          "value": {
            "binding": { "origin": "root", "pointer": "/billing_cycle" },
            "widget": "daisyui:radio"
          }
        },
        {
          "type": "control",
          "value": {
            "binding": { "origin": "root", "pointer": "/region" },
            "widget": "daisyui:select"
          }
        }
      ]
    }
  }
}"#;

/// A form bound through the daisyUI control registry: every control kind is a daisyUI field.
/// Strings, numbers, and integers are `Input`s; a non-nullable boolean is a native checkbox, a
/// nullable one the registry `Checkbox` with null as indeterminate, a write-only one a
/// replacement select; choices are a native select, a radio group, or the compound select as the
/// UI schema asks; constants are read-only output. Only the layout and the submit button come
/// from the built-ins.
#[component]
pub fn DaisyuiControlsExample() -> Element {
    let definition = use_hook(definition);
    let form = use_form(
        definition,
        json!({
            "name": "Ada",
            "age": 36,
            "price": 19.5,
            "active": true,
            "newsletter": null,
            "two_factor": true,
            "plan": "team",
            "billing_cycle": "yearly",
            "region": "eu",
            "recovery_channel": "email",
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
                    for secret in ["access_token", "two_factor", "recovery_channel"] {
                        if let Some(value) = displayed.get_mut(secret) {
                            *value = serde_json::Value::String("[redacted]".to_owned());
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
    let ui_schema = parse_ui_schema_v1(UI_SCHEMA.as_bytes(), &CompilationProfile::default())
        .expect("the daisyUI example UI schema should parse");
    FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": [
            "name", "age", "price", "active", "two_factor", "plan", "region",
            "recovery_channel", "account_type"
        ],
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
            "newsletter": {
                "type": ["boolean", "null"],
                "title": "Product newsletter",
                "description": "Null means the account holder has not decided yet."
            },
            "two_factor": {
                "type": "boolean",
                "title": "Two-factor authentication",
                "writeOnly": true
            },
            "plan": {
                "title": "Plan",
                "enum": ["starter", "team", "enterprise"]
            },
            "billing_cycle": {
                "type": ["string", "null"],
                "title": "Billing cycle",
                "description": "Null keeps the plan's default cycle.",
                "enum": ["monthly", "yearly", null]
            },
            "region": {
                "title": "Data region",
                "enum": ["eu", "us", "apac"]
            },
            "recovery_channel": {
                "title": "Recovery channel",
                "enum": ["email", "sms"],
                "writeOnly": true
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
    .ui_schema(ui_schema)
    .compile()
    .expect("the daisyUI example schemas should compile")
}

#[cfg(test)]
mod tests {
    use dioxus::core::{NoOpMutations, VirtualDom};

    #[test]
    fn example_schemas_compile() {
        super::definition();
    }

    /// The registry binds every control the example authors, and the two widget symbols reach
    /// their widgets; a bind failure would surface as a rendered panic rather than a form.
    #[test]
    fn the_example_form_binds_and_renders_every_widget() {
        let mut dom = VirtualDom::new(super::DaisyuiControlsExample);
        dom.rebuild_in_place();
        for _ in 0..4 {
            dom.render_immediate(&mut NoOpMutations);
        }
        let html = dioxus_ssr::render(&dom);

        assert!(!html.contains("Encountered panic"), "{html}");
        assert!(html.contains("role=\"radiogroup\""), "{html}");
        assert!(html.contains("aria-haspopup=\"listbox\""), "{html}");
        assert!(html.contains("role=\"checkbox\""), "{html}");
        assert!(html.contains("data-write-only-replacement"), "{html}");
        assert!(
            html.contains("data-schemaform-control=\"constant\""),
            "{html}"
        );
        assert!(!html.contains("class=\"schemaform-control\""), "{html}");
    }
}
