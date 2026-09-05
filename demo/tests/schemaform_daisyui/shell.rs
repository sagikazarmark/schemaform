//! The daisyUI form shell.

use crate::support::{RenderedForm, arrays_app};

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
