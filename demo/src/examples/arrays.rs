use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{SchemaForm, RenderConfiguration, use_form};
use serde_json::json;

use crate::components::StatusLine;

/// Both arrays are homogeneous. The adapter exposes identity-based append,
/// insert, remove, and move actions while the core preserves item state as rows
/// move.
#[component]
pub fn ArraysExample() -> Element {
    let definition = use_hook(definition);
    let form = use_form(
        definition,
        json!({
            "tags": ["rust", "dioxus"],
            "team": [
                { "name": "Ada", "role": "engineering" },
                { "name": "Lin", "role": "design" }
            ]
        }),
    )
    .expect("the array example form should be created");
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::default()
            .bind(&bound_form)
            .expect("array controls should bind")
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
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["tags", "team"],
        "properties": {
            "tags": {
                "type": "array",
                "title": "Tags",
                "minItems": 1,
                "maxItems": 5,
                "items": {
                    "type": "string",
                    "default": "new-tag"
                }
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
            }
        }
    }))
    .expect("the array example schema should compile")
}

#[cfg(test)]
mod tests {
    #[test]
    fn example_schema_compiles() {
        super::definition();
    }
}
