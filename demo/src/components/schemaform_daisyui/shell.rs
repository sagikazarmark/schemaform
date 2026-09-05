//! The daisyUI form shell: the finding summary, the body, and the submit button inside the
//! adapter-owned form element.

use dioxus::prelude::*;
use schemaform_dioxus::{ShellContext, ShellRenderer};

use super::Appearance;
use crate::components::button::{Button, ButtonColor};

/// Lays the form's contents out as a vertical grid — the finding summary, then the body, then a
/// primary daisyUI button for the submit affordance.
///
/// The button is `type="submit"`, so it submits through the adapter-owned form element and
/// pressing Enter in a text control takes the same path. The summary arrives with its
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
        let appearance = self.appearance;
        rsx! {
            div { class: appearance.utilities("grid gap-4"), "data-schemaform-daisyui": "shell",
                {context.summary}
                {context.body}
                Button {
                    id: submit.id.clone(),
                    r#type: "submit",
                    color: ButtonColor::Primary,
                    class: appearance.utilities("w-fit"),
                    "{submit.label}"
                }
            }
        }
    }
}
