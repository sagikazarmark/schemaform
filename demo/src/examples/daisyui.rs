use dioxus::prelude::*;
use schemaform::{CompilationProfile, FormDefinition, json::parse_ui_schema_v1};
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

use crate::components::{StatusLine, schemaform_daisyui};

/// The authored presentation: four tabs over the account. The identity group lays its controls
/// out on a grid, the billing cycle asks for the `daisyui:radio` widget and the region for
/// `daisyui:select`, and every other node is generated from the data schema in place: the billing
/// address as a fixed object with its presence operations, the team and tags as homogeneous
/// arrays with their item chrome.
const UI_SCHEMA: &str = r#"{
  "version": 1,
  "root": {
    "type": "tabs",
    "value": {
      "panels": [
        {
          "title": { "fallback": "Profile" },
          "child": {
            "type": "stack",
            "value": {
              "children": [
                {
                  "type": "group",
                  "value": {
                    "title": { "fallback": "Identity" },
                    "child": {
                      "type": "grid",
                      "value": {
                        "cells": [
                          {
                            "compact_span": 12,
                            "wide_span": 6,
                            "child": {
                              "type": "control",
                              "value": { "binding": { "origin": "root", "pointer": "/name" } }
                            }
                          },
                          {
                            "compact_span": 12,
                            "wide_span": 6,
                            "child": {
                              "type": "control",
                              "value": { "binding": { "origin": "root", "pointer": "/nickname" } }
                            }
                          },
                          {
                            "compact_span": 12,
                            "wide_span": 6,
                            "child": {
                              "type": "control",
                              "value": { "binding": { "origin": "root", "pointer": "/age" } }
                            }
                          }
                        ]
                      }
                    }
                  }
                },
                {
                  "type": "auto",
                  "value": {
                    "binding": { "origin": "root", "pointer": "" },
                    "properties": {
                      "include": ["active", "newsletter", "account_type", "customer_id"],
                      "order": [
                        { "property": "active" },
                        { "property": "newsletter" },
                        { "property": "account_type" },
                        { "property": "customer_id" }
                      ]
                    }
                  }
                }
              ]
            }
          }
        },
        {
          "title": { "fallback": "Billing" },
          "child": {
            "type": "stack",
            "value": {
              "children": [
                {
                  "type": "auto",
                  "value": {
                    "binding": { "origin": "root", "pointer": "" },
                    "properties": { "include": ["plan"] }
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
                },
                {
                  "type": "auto",
                  "value": {
                    "binding": { "origin": "root", "pointer": "" },
                    "properties": {
                      "include": ["price", "billing_address"],
                      "order": [{ "property": "price" }, { "property": "billing_address" }]
                    }
                  }
                }
              ]
            }
          }
        },
        {
          "title": { "fallback": "Security" },
          "child": {
            "type": "auto",
            "value": {
              "binding": { "origin": "root", "pointer": "" },
              "properties": {
                "include": ["two_factor", "recovery_channel", "access_token"],
                "order": [
                  { "property": "two_factor" },
                  { "property": "recovery_channel" },
                  { "property": "access_token" }
                ]
              }
            }
          }
        },
        {
          "title": { "fallback": "Team" },
          "child": {
            "type": "auto",
            "value": {
              "binding": { "origin": "root", "pointer": "" },
              "properties": {
                "include": ["team", "tags"],
                "order": [{ "property": "team" }, { "property": "tags" }]
              }
            }
          }
        }
      ]
    }
  }
}"#;

/// The writing direction the example renders in.
///
/// The direction is an attribute on the wrapper around the form, not a property of the definition
/// or the form data: the RTL variant is the same form with its chrome mirrored.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WritingDirection {
    #[default]
    Ltr,
    Rtl,
}

impl WritingDirection {
    /// The value of the HTML `dir` attribute for this direction.
    pub fn as_attribute(self) -> &'static str {
        match self {
            Self::Ltr => "ltr",
            Self::Rtl => "rtl",
        }
    }
}

/// A form bound through every seam the daisyUI component fills. Its control registry makes every
/// control kind a daisyUI field: strings, numbers, and integers are `Input`s; a non-nullable
/// boolean is a native checkbox, a nullable one the registry `Checkbox` with null as
/// indeterminate, a write-only one a replacement select; choices are a native select, a radio
/// group, or the compound select as the UI schema asks; constants are read-only output. Its
/// structure bundle renders the arrays as collections of item cards with joined action buttons
/// and the form shell with a primary submit button, and its finding presenter frames the summary
/// as an alert.
///
/// The structure no seam exists for yet — tabs, the authored group, the grid, the fixed object
/// and its presence operations — is the built-in renderer's, styled with daisyUI classes through
/// its `schemaform-*` class hooks by the demo stylesheet.
#[component]
pub fn DaisyuiFormExample(#[props(default)] direction: WritingDirection) -> Element {
    let definition = use_hook(definition);
    let form = use_form(definition, baseline_form_data())
        .expect("the daisyUI example form should be created");
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .controls(schemaform_daisyui::controls())
            .structure(schemaform_daisyui::structure())
            .summary_presenter(schemaform_daisyui::findings())
            .local_presenter(schemaform_daisyui::findings())
            .build()
            .bind(&bound_form)
            .expect("the daisyUI registry should bind every control")
    });
    let mut submitted = use_signal(String::new);
    let reset_form = form.clone();

    rsx! {
        div { class: "space-y-4", dir: direction.as_attribute(),
            SchemaForm {
                form: bound,
                on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                    submitted.set(redacted_submission_text(snapshot.form_data()));
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

/// The submitted form data as the page displays it: pretty-printed JSON with the write-only
/// values redacted, so a secret never shows up on the page.
pub fn redacted_submission_text(form_data: &serde_json::Value) -> String {
    let mut displayed = form_data.clone();
    for secret in ["access_token", "two_factor", "recovery_channel"] {
        if let Some(value) = displayed.get_mut(secret) {
            *value = serde_json::Value::String("[redacted]".to_owned());
        }
    }
    serde_json::to_string_pretty(&displayed).expect("form data should serialize")
}

/// The baseline form data. The billing address is present so its group renders with a remove
/// operation; the arrays start with two items each so every item operation has a target.
pub fn baseline_form_data() -> serde_json::Value {
    json!({
        "name": "Ada",
        "nickname": null,
        "age": 36,
        "active": true,
        "newsletter": null,
        "account_type": "standard",
        "customer_id": "cus_1843",
        "plan": "team",
        "billing_cycle": "yearly",
        "region": "eu",
        "price": 19.5,
        "billing_address": {
            "street": "12 Analytical Row",
            "city": "London",
            "postal_code": "N1 9GU"
        },
        "two_factor": true,
        "recovery_channel": "email",
        "access_token": "not-rendered",
        "team": [
            { "name": "Ada", "role": "engineering" },
            { "name": "Lin", "role": "design" }
        ],
        "tags": ["rust", "dioxus"]
    })
}

/// The compiled data schema and UI schema. Shared with the built-in comparison page, which
/// renders exactly this definition and baseline through the built-in renderer.
pub fn definition() -> FormDefinition {
    let ui_schema = parse_ui_schema_v1(UI_SCHEMA.as_bytes(), &CompilationProfile::default())
        .expect("the daisyUI example UI schema should parse");
    FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": [
            "name", "age", "active", "account_type", "plan", "region", "price",
            "two_factor", "recovery_channel", "team", "tags"
        ],
        "properties": {
            "name": {
                "type": "string",
                "title": "Display name",
                "description": "At least three characters.",
                "minLength": 3
            },
            "nickname": {
                "type": ["string", "null"],
                "title": "Nickname"
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
            "newsletter": {
                "type": ["boolean", "null"],
                "title": "Product newsletter",
                "description": "Null means the account holder has not decided yet."
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
            "price": {
                "type": "number",
                "title": "Monthly price",
                "description": "In euros, decimals allowed.",
                "minimum": 0
            },
            "billing_address": {
                "type": "object",
                "title": "Billing address",
                "description": "Optional. Remove it to bill the account holder directly.",
                "additionalProperties": false,
                "required": ["street", "city"],
                "properties": {
                    "street": { "type": "string", "title": "Street", "minLength": 1 },
                    "city": { "type": "string", "title": "City", "minLength": 1 },
                    "postal_code": { "type": "string", "title": "Postal code" }
                }
            },
            "two_factor": {
                "type": "boolean",
                "title": "Two-factor authentication",
                "writeOnly": true
            },
            "recovery_channel": {
                "title": "Recovery channel",
                "enum": ["email", "sms"],
                "writeOnly": true
            },
            "access_token": {
                "type": "string",
                "title": "Access token",
                "writeOnly": true
            },
            "team": {
                "type": "array",
                "title": "Team members",
                "minItems": 1,
                "maxItems": 4,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name", "role"],
                    "properties": {
                        "name": {
                            "type": "string",
                            "title": "Name",
                            "default": "New teammate"
                        },
                        "role": {
                            "title": "Role",
                            "enum": ["engineering", "design", "operations"],
                            "default": "engineering"
                        }
                    }
                }
            },
            "tags": {
                "type": "array",
                "title": "Tags",
                "minItems": 1,
                "maxItems": 5,
                "items": {
                    "type": "string",
                    "default": "new-tag"
                }
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

    use super::{DaisyuiFormExample, DaisyuiFormExampleProps, WritingDirection};

    /// Mounts the example as the browser would and returns the markup it settles on.
    fn render(direction: WritingDirection) -> String {
        let mut dom =
            VirtualDom::new_with_props(DaisyuiFormExample, DaisyuiFormExampleProps { direction });
        dom.rebuild_in_place();
        for _ in 0..4 {
            dom.render_immediate(&mut NoOpMutations);
        }
        let html = dioxus_ssr::render(&dom);
        assert!(!html.contains("Encountered panic"), "{html}");
        html
    }

    #[test]
    fn example_schemas_compile() {
        super::definition();
    }

    /// The registry binds every control the example authors, and the two widget symbols reach
    /// their widgets; a bind failure would surface as a rendered panic rather than a form.
    #[test]
    fn the_example_form_binds_and_renders_every_widget() {
        let html = render(WritingDirection::default());

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

    /// The structure the component renders through the adapter's seams is on the page: the arrays
    /// as daisyUI collections with card items and joined action buttons, the submit button as a
    /// primary daisyUI button, and the finding summary placed by the shell; none of the built-in
    /// array chrome remains. The tabs, the authored group, and the fixed object are still the
    /// built-in renderer's, themed through their class hooks.
    #[test]
    fn the_example_form_renders_daisyui_arrays_and_shell_around_themed_built_in_structure() {
        let html = render(WritingDirection::default());

        assert!(html.contains("class=\"schemaform-tabs\""), "{html}");
        assert!(html.contains("role=\"tablist\""), "{html}");
        assert!(
            html.contains("class=\"schemaform-group schemaform-authored-group\""),
            "{html}"
        );
        assert!(
            html.contains("class=\"schemaform-group schemaform-fixed-object\""),
            "{html}"
        );
        assert!(html.contains("data-remove-value"), "{html}");

        assert!(html.contains("data-schemaform-daisyui=\"shell\""), "{html}");
        assert!(html.contains("data-finding-summary"), "{html}");
        assert!(html.contains("type=\"submit\""), "{html}");
        assert!(html.contains("btn-primary"), "{html}");
        assert_eq!(
            html.matches("data-schemaform-daisyui=\"collection\"")
                .count(),
            2,
            "{html}"
        );
        assert_eq!(
            html.matches("data-schemaform-daisyui=\"collection-item\"")
                .count(),
            4,
            "two team members and two tags: {html}"
        );
        assert!(
            html.contains("aria-label=\"Move Team members item at position 1 down\""),
            "{html}"
        );
        assert!(
            !html.contains("schemaform-group schemaform-array"),
            "{html}"
        );
        assert!(!html.contains("data-append-item"), "{html}");
        assert!(!html.contains("data-move-item-down"), "{html}");
        // The generated regions follow the authored order, not the data schema's key order.
        assert!(
            html.find(">Team members</legend>") < html.find(">Tags</legend>"),
            "{html}"
        );
    }

    /// The direction lives on the example's wrapper, so the RTL variant mirrors the daisyUI
    /// chrome without touching the definition, the form data, or the registry.
    #[test]
    fn the_direction_is_set_on_the_form_wrapper() {
        let ltr = render(WritingDirection::Ltr);
        assert!(ltr.contains("dir=\"ltr\""), "{ltr}");
        assert!(!ltr.contains("dir=\"rtl\""), "{ltr}");
        assert!(render(WritingDirection::Rtl).contains("dir=\"rtl\""));
    }
}
