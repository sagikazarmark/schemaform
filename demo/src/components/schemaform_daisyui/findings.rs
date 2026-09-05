//! The daisyUI finding presenter: the form-wide summary as an alert whose findings reveal and
//! focus their targets, and node-local findings as a stack of descriptions.

use dioxus::prelude::*;
use schemaform_dioxus::render::{FindingCollectionContext, FindingCollectionPresenter};

use super::Appearance;
use super::parts::icon;

/// Heroicons' outline exclamation triangle, marking the summary alert.
const WARNING_ICON: &str = "M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z";

/// Presents a finding collection with daisyUI.
///
/// The form-wide summary is a soft `alert` — `alert-error` while any finding blocks submission,
/// `alert-warning` when every finding is advisory — listing one `link` button per finding that
/// reveals and focuses the finding's target. An empty summary renders nothing, so the
/// adapter-owned region stays empty rather than framing nothing. A node-local collection is a
/// stack of descriptions in the error or warning colour.
///
/// Every finding's container carries the adapter's stable id, so ids the adapter hands out in
/// `aria-describedby` resolve, plus `data-finding` (the code) and `data-blocking`. The
/// [`Appearance`] axis switches the layout and colour utilities off; the `alert` and `link`
/// component classes always render.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaisyuiFindings {
    appearance: Appearance,
}

impl DaisyuiFindings {
    /// The same presenter at `appearance`.
    pub fn appearance(self, appearance: Appearance) -> Self {
        Self { appearance }
    }
}

impl FindingCollectionPresenter for DaisyuiFindings {
    fn render(&self, context: FindingCollectionContext) -> Element {
        if context.is_summary() {
            summary(context, self.appearance)
        } else {
            local(context, self.appearance)
        }
    }
}

fn summary(context: FindingCollectionContext, appearance: Appearance) -> Element {
    let entries = context
        .entries()
        .map(|entry| (entry.finding().clone(), entry.target_focus().clone()))
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return rsx! {};
    }
    let blocking = entries.iter().any(|(finding, _)| finding.blocking);
    let tone = if blocking {
        "alert-error"
    } else {
        "alert-warning"
    };
    let alert_class = appearance.utilities("items-start");
    let list_class = appearance.utilities("flex min-w-0 flex-col gap-1");
    let link_class = appearance.utilities("text-start");
    rsx! {
        div { class: "alert alert-soft {tone} {alert_class}",
            {icon(WARNING_ICON, appearance.utilities("size-5 shrink-0"))}
            ul { class: list_class, role: "list",
                for (finding, target_focus) in entries {
                    li {
                        key: "{finding.stable_id}",
                        id: finding.stable_id.clone(),
                        "data-finding": finding.code.clone(),
                        "data-blocking": finding.blocking.to_string(),
                        button {
                            r#type: "button",
                            class: "link {link_class}",
                            onclick: move |_| target_focus.focus(),
                            "{finding.text}"
                        }
                    }
                }
            }
        }
    }
}

fn local(context: FindingCollectionContext, appearance: Appearance) -> Element {
    let findings = context.findings().cloned().collect::<Vec<_>>();
    rsx! {
        for finding in findings {
            p {
                key: "{finding.stable_id}",
                id: finding.stable_id.clone(),
                class: appearance.utilities(if finding.blocking { "min-w-0 text-error" } else { "min-w-0 text-warning" }),
                "data-finding": finding.code.clone(),
                "data-blocking": finding.blocking.to_string(),
                "{finding.text}"
            }
        }
    }
}
