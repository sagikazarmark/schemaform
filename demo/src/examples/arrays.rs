use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

use crate::components::{StatusLine, schemaform_daisyui};

/// Both arrays are homogeneous. The adapter exposes identity-based append,
/// insert, remove, and move operations while the core preserves item state as
/// rows move. The tags may be emptied; the team keeps at least one member.
///
/// The chrome is the daisyUI component's `CollectionRenderer`: each item is a
/// card whose affordances are joined square buttons named by their position,
/// and an emptied collection shows its empty state. The adapter keeps item
/// identity, keying, focus after a mutation, and the live region; the renderer
/// only places what it is handed.
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
        RenderConfiguration::builder()
            .controls(schemaform_daisyui::controls())
            .structure(schemaform_daisyui::structure())
            .summary_presenter(schemaform_daisyui::findings())
            .local_presenter(schemaform_daisyui::findings())
            .build()
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
    use dioxus::core::{NoOpMutations, VirtualDom};

    /// Mounts the example as the browser would and returns the markup it settles on.
    fn render() -> String {
        let mut dom = VirtualDom::new(super::ArraysExample);
        dom.rebuild_in_place();
        for _ in 0..4 {
            dom.render_immediate(&mut NoOpMutations);
        }
        let html = dioxus_ssr::render(&dom);
        assert!(!html.contains("Encountered panic"), "{html}");
        html
    }

    #[test]
    fn example_schema_compiles() {
        super::definition();
    }

    /// Both arrays render through the daisyUI collection: two collections, four item cards, the
    /// item actions named by position, and none of the built-in array chrome.
    #[test]
    fn the_example_renders_both_arrays_as_daisyui_collections() {
        let html = render();

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
            "{html}"
        );
        assert!(
            html.contains("aria-label=\"Insert Tags item before position 1\""),
            "{html}"
        );
        assert!(
            html.contains("aria-label=\"Remove Team members item at position 2\""),
            "{html}"
        );
        assert!(html.contains(">Add Tags item</button>"), "{html}");
        assert!(html.contains("data-schemaform-daisyui=\"shell\""), "{html}");
        assert!(
            !html.contains("schemaform-group schemaform-array"),
            "{html}"
        );
        assert!(!html.contains("data-append-item"), "{html}");
    }
}
