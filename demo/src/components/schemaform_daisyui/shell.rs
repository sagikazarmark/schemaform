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
