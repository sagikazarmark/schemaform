//! The daisyUI form shell: the body, the submit button, and the finding summary inside the
//! adapter-owned form element.

use dioxus::prelude::*;
use schemaform_dioxus::{ShellContext, ShellRenderer};

use super::Appearance;
use crate::components::button::{Button, ButtonColor};

/// Lays the form's contents out as a vertical grid — the body, then a primary daisyUI button for
/// the submit affordance, then the finding summary.
///
/// The summary comes last so that its growth never moves anything in the form. A finding is
/// revealed when its control is left, and leaving a control is the `mousedown` of whatever is
/// clicked next; a summary above the body would push that target down between `mousedown` and
/// `mouseup`, and the click would land on nothing. A blocked submission still reaches the summary
/// here, because the adapter focuses it.
///
/// The button is `type="submit"`, so it submits through the adapter-owned form element and
/// pressing Enter in a text control takes the same path. A read-only form gets no button. The summary arrives with its
/// adapter-owned region wrapper; this component's finding presenter ([`super::findings`]) is what
/// frames the findings inside it as an alert, since the shell cannot see whether there are any.
/// The [`Appearance`] axis switches the grid and the button's width utility off.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaisyuiShell {
    appearance: Appearance,
}

impl DaisyuiShell {
    /// The same shell at `appearance`.
    pub fn appearance(self, appearance: Appearance) -> Self {
        Self { appearance }
    }
}

impl ShellRenderer for DaisyuiShell {
    fn shell(&self, context: ShellContext) -> Element {
        let submit = context.submit;
        let read_only = context.read_only;
        let appearance = self.appearance;
        rsx! {
            div { class: appearance.utilities("grid gap-4"), "data-schemaform-daisyui": "shell",
                if let Some(id) = context.heading_id {
                    h2 { id, class: appearance.utilities("text-xl font-semibold"), "{context.presentation.label}" }
                }
                if let Some(help) = context.presentation.help {
                    p { id: help.id, "{help.text}" }
                }
                {context.body}
                // A read-only form submits nothing, so it offers no button.
                if !read_only {
                    Button {
                        id: submit.id.clone(),
                        r#type: "submit",
                        color: ButtonColor::Primary,
                        class: appearance.utilities("w-fit"),
                        "{submit.label}"
                    }
                }
                {context.summary}
            }
        }
    }
}
