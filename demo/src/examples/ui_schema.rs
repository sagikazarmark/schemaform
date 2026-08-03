use dioxus::prelude::*;
use schemaform::{CompilationProfile, FormDefinition, json::parse_ui_schema_v1};
use schemaform_dioxus::{SchemaForm, RenderConfiguration, use_form};
use serde_json::json;

use crate::components::StatusLine;

const UI_SCHEMA: &str = r#"{
  "version": 1,
  "root": {
    "type": "stack",
    "value": {
      "id": "profile-stack",
      "children": [
        {
          "type": "text",
          "value": {
            "content": {
              "fallback": "Tell us how your team should be configured."
            }
          }
        },
        {
          "type": "grid",
          "value": {
            "cells": [
              {
                "compact_span": 12,
                "wide_span": 6,
                "child": {
                  "type": "control",
                  "value": {
                    "binding": { "origin": "root", "pointer": "/first_name" }
                  }
                }
              },
              {
                "compact_span": 12,
                "wide_span": 6,
                "child": {
                  "type": "control",
                  "value": {
                    "binding": { "origin": "root", "pointer": "/last_name" }
                  }
                }
              }
            ]
          }
        },
        {
          "type": "group",
          "value": {
            "title": { "fallback": "Contact" },
            "child": {
              "type": "stack",
              "value": {
                "children": [
                  {
                    "type": "control",
                    "value": {
                      "binding": { "origin": "root", "pointer": "/email" }
                    }
                  },
                  {
                    "type": "control",
                    "value": {
                      "binding": { "origin": "root", "pointer": "/company" }
                    }
                  }
                ]
              }
            }
          }
        },
        {
          "type": "tabs",
          "value": {
            "panels": [
              {
                "title": { "fallback": "Plan" },
                "child": {
                  "type": "control",
                  "value": {
                    "binding": { "origin": "root", "pointer": "/plan" },
                    "label": {
                      "value": { "fallback": "Workspace plan" }
                    }
                  }
                }
              },
              {
                "title": { "fallback": "Preferences" },
                "child": {
                  "type": "control",
                  "value": {
                    "binding": { "origin": "root", "pointer": "/newsletter" }
                  }
                }
              }
            ]
          }
        }
      ]
    }
  }
}"#;

/// The independent UI schema chooses ordering and layout without weakening the
/// data schema's validation or changing submitted form data.
#[component]
pub fn AuthoredUiSchemaExample() -> Element {
    let definition = use_hook(definition);
    let form = use_form(
        definition,
        json!({
            "first_name": "Ada",
            "last_name": "Lovelace",
            "email": "ada@example.com",
            "company": "Analytical Engines",
            "plan": "team",
            "newsletter": true
        }),
    )
    .expect("the authored example form should be created");
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::default()
            .bind(&bound_form)
            .expect("authored controls should bind")
    });
    let mut submitted = use_signal(String::new);

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                submitted.set(
                    serde_json::to_string_pretty(snapshot.form_data())
                        .expect("form data should serialize"),
                );
            },
            on_error: move |error| eprintln!("form operation failed: {error}"),
        }
        StatusLine { status: submitted.read().clone() }
    }
}

fn definition() -> FormDefinition {
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["first_name", "last_name", "email", "plan", "newsletter"],
        "properties": {
            "first_name": { "type": "string", "title": "First name", "minLength": 1 },
            "last_name": { "type": "string", "title": "Last name", "minLength": 1 },
            "email": { "type": "string", "title": "Email address", "minLength": 3 },
            "company": { "type": "string", "title": "Company" },
            "plan": { "title": "Plan", "enum": ["starter", "team", "enterprise"] },
            "newsletter": { "type": "boolean", "title": "Product updates" }
        }
    });
    let ui_schema = parse_ui_schema_v1(UI_SCHEMA.as_bytes(), &CompilationProfile::default())
        .expect("the authored UI schema should parse");
    let definition = FormDefinition::compiler(data_schema)
        .ui_schema(ui_schema)
        .compile()
        .expect("the authored UI schema should compile");
    definition
}

#[cfg(test)]
mod tests {
    #[test]
    fn example_schemas_compile() {
        super::definition();
    }
}
