//! The daisyUI form shell: the finding summary, the body, and the submit button inside the
//! adapter-owned form element.

use dioxus::prelude::*;
use schemaform_dioxus::{ShellContext, ShellRenderer};

use crate::components::button::{Button, ButtonColor};

/// Lays the form's contents out as a vertical grid — the finding summary, then the body, then a
/// primary daisyUI button for the submit affordance.
///
/// The button is `type="submit"`, so it submits through the adapter-owned form element and
/// pressing Enter in a text control takes the same path. The summary arrives with its
/// adapter-owned region wrapper; this component's finding presenter ([`super::findings`]) is what
/// frames the findings inside it as an alert, since the shell cannot see whether there are any.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaisyuiShell;

impl ShellRenderer for DaisyuiShell {
    fn shell(&self, context: ShellContext) -> Element {
        let submit = context.submit;
        rsx! {
            div { class: "grid gap-4", "data-schemaform-daisyui": "shell",
                {context.summary}
                {context.body}
                Button {
                    id: submit.id.clone(),
                    r#type: "submit",
                    color: ButtonColor::Primary,
                    class: "w-fit",
                    "{submit.label}"
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::components::schemaform_daisyui::test_support::{RenderedForm, arrays_app};

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
}
