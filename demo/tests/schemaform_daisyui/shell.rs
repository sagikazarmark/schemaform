//! The daisyUI form shell, and the component that binds a form through every seam.

use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::use_form;
use serde_json::json;

use crate::support::{RenderedForm, TestAppProps, arrays_app};
use demo::components::schemaform_daisyui::SchemaformDaisyui;

fn mount() -> RenderedForm {
    RenderedForm::mount(arrays_app)
}

/// The shell keeps the adapter's contract — summary first, then the body — and places the
/// submit affordance as a primary daisyUI button that submits through the form element, so
/// Enter in a text control and clicking the button take the same path.
#[test]
fn the_shell_places_summary_then_body_and_a_primary_submit_button() {
    let rendered = mount();
    let html = rendered.html();

    let form = rendered
        .find(|tag| tag.element == "form")
        .expect("the adapter's form element should be rendered");
    let form_id = form.attribute("id").expect("the form carries its id");
    let shell = rendered
        .find(|tag| tag.attribute("data-schemaform-daisyui") == Some("shell"))
        .expect("the daisyUI shell should wrap the form contents");
    assert!(shell.has_classes(&["grid", "gap-4"]), "{shell:?}");

    let submit = rendered
        .by_id(&format!("{form_id}-submit"))
        .expect("the submit affordance should carry its id");
    assert_eq!(submit.element, "button");
    assert_eq!(submit.attribute("type"), Some("submit"));
    assert!(submit.has_classes(&["btn", "btn-primary"]), "{submit:?}");
    assert!(html.contains(">Submit</button>"), "{html}");

    let summary = html
        .find("data-finding-summary")
        .expect("the summary region should be placed");
    let body = html
        .find("name=\"/name\"")
        .expect("the body should be placed");
    let submit = html
        .find(&format!("id=\"{form_id}-submit\""))
        .expect("checked above");
    assert!(summary < body && body < submit, "{html}");
}

/// A form rendered through `SchemaformDaisyui` rather than a hand-composed configuration.
fn component_app(props: TestAppProps) -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "required": ["name"],
            "properties": {
                "name": { "type": "string", "title": "Name", "minLength": 2 },
                "tags": {
                    "type": "array",
                    "title": "Tags",
                    "items": { "type": "string", "title": "Tag" }
                }
            }
        }))
        .expect("the component data schema should compile")
    });
    let form = use_form(definition, json!({ "name": "Ada", "tags": ["rust"] }))
        .expect("the component form should be created");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    rsx! {
        SchemaformDaisyui { form, on_submit: move |_| {} }
    }
}

/// The component binds the form through every seam at once: the shell, the collection, the
/// control renderer, and — once a finding is visible — the presenter.
#[test]
fn the_component_binds_a_form_through_every_daisyui_seam() {
    let mut rendered = RenderedForm::mount(component_app);

    assert!(
        rendered
            .find(|tag| tag.attribute("data-schemaform-daisyui") == Some("shell"))
            .is_some(),
        "the shell is the daisyUI shell"
    );
    assert!(
        rendered
            .find(|tag| tag.attribute("data-schemaform-daisyui") == Some("collection"))
            .is_some(),
        "the array is the daisyUI collection"
    );
    let name = rendered.control("/name");
    assert!(
        name.classes().contains(&"input"),
        "the control is a daisyUI input: {name:?}"
    );

    let actions = rendered.actions_at("/name");
    actions.input_text("A").expect("the edit should apply");
    actions.blur().expect("leaving the control should apply");
    rendered.settle();
    assert!(
        rendered
            .find(|tag| tag.has_classes(&["alert", "alert-error"]))
            .is_some(),
        "the summary is the daisyUI alert"
    );
}
