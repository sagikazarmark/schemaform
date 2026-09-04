//! The daisyUI finding presenter: the form-wide summary as an alert whose findings reveal and
//! focus their targets, and node-local findings as a stack of descriptions.

use std::sync::Arc;

use dioxus::prelude::*;
use schemaform_dioxus::render::{FindingCollectionContext, FindingCollectionPresenter};

use super::parts::icon;

/// Heroicons' outline exclamation triangle, marking the summary alert.
const WARNING_ICON: &str = "M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126ZM12 15.75h.007v.008H12v-.008Z";

/// The finding presenter for both presenter slots of a render configuration.
///
/// Wire it as the summary presenter for the alert and as the local presenter so the findings a
/// built-in container or this component's collection renders through
/// `NodePresentation::present_findings` are daisyUI-styled as well.
pub fn findings() -> Arc<dyn FindingCollectionPresenter> {
    Arc::new(DaisyuiFindings)
}

/// Presents a finding collection with daisyUI.
///
/// The form-wide summary is a soft `alert` — `alert-error` while any finding blocks submission,
/// `alert-warning` when every finding is advisory — listing one `link` button per finding that
/// reveals and focuses the finding's target. An empty summary renders nothing, so the
/// adapter-owned region stays empty rather than framing nothing. A node-local collection is a
/// stack of descriptions in the error or warning colour.
///
/// Every finding's container carries the adapter's stable id, so ids the adapter hands out in
/// `aria-describedby` resolve, plus `data-finding` (the code) and `data-blocking`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaisyuiFindings;

impl FindingCollectionPresenter for DaisyuiFindings {
    fn render(&self, context: FindingCollectionContext) -> Element {
        if context.is_summary() {
            summary(context)
        } else {
            local(context)
        }
    }
}

fn summary(context: FindingCollectionContext) -> Element {
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
    rsx! {
        div { class: "alert alert-soft {tone} items-start",
            {icon(WARNING_ICON, "size-5 shrink-0")}
            ul { class: "flex min-w-0 flex-col gap-1", role: "list",
                for (finding, target_focus) in entries {
                    li {
                        key: "{finding.stable_id}",
                        id: finding.stable_id.clone(),
                        "data-finding": finding.code.clone(),
                        "data-blocking": finding.blocking.to_string(),
                        button {
                            r#type: "button",
                            class: "link text-start",
                            onclick: move |_| target_focus.focus(),
                            "{finding.text}"
                        }
                    }
                }
            }
        }
    }
}

fn local(context: FindingCollectionContext) -> Element {
    let findings = context.findings().cloned().collect::<Vec<_>>();
    rsx! {
        for finding in findings {
            p {
                key: "{finding.stable_id}",
                id: finding.stable_id.clone(),
                class: if finding.blocking { "min-w-0 text-error" } else { "min-w-0 text-warning" },
                "data-finding": finding.code.clone(),
                "data-blocking": finding.blocking.to_string(),
                "{finding.text}"
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::components::schemaform_daisyui::test_support::{
        RenderedForm, arrays_app, inner_html, tags,
    };

    fn mount() -> RenderedForm {
        RenderedForm::mount(arrays_app)
    }

    /// The HTML inside the adapter-owned summary region.
    fn summary_html(rendered: &RenderedForm) -> String {
        let html = rendered.html();
        let region = rendered
            .find(|tag| tag.attribute("data-finding-summary").is_some())
            .expect("the summary region should be placed");
        inner_html(
            &html,
            region.attribute("id").expect("the region carries its id"),
        )
    }

    /// A summary with nothing to say renders nothing: the region stays empty rather than showing
    /// an empty alert.
    #[test]
    fn an_empty_summary_renders_no_alert() {
        let rendered = mount();

        let summary = summary_html(&rendered);
        assert!(
            !tags(&summary)
                .iter()
                .any(|tag| tag.classes().contains(&"alert")),
            "{summary}"
        );
    }

    /// A visible finding frames the summary as a soft error alert listing one button per finding;
    /// the button carries the finding's localized text, and its container the finding's stable id
    /// and code, as the built-in presenter's does.
    #[test]
    fn a_visible_finding_frames_the_summary_as_an_error_alert_with_focus_buttons() {
        let mut rendered = mount();
        let name = rendered.actions_at("/name");
        name.input_text("A").expect("the edit should apply");
        name.blur().expect("leaving the control should apply");
        rendered.settle();

        let html = rendered.html();
        let summary = summary_html(&rendered);
        let alert = tags(&summary)
            .into_iter()
            .find(|tag| tag.classes().contains(&"alert"))
            .unwrap_or_else(|| panic!("the summary should be an alert:\n{html}"));
        assert!(
            alert.has_classes(&["alert", "alert-error", "alert-soft"]),
            "{alert:?}"
        );
        assert_eq!(
            alert.attribute("role"),
            None,
            "the adapter's region already names the summary: {alert:?}"
        );

        let findings = tags(&summary)
            .into_iter()
            .filter(|tag| tag.attribute("data-finding").is_some())
            .collect::<Vec<_>>();
        assert_eq!(findings.len(), 1, "{summary}");
        let finding = &findings[0];
        assert_eq!(finding.attribute("data-blocking"), Some("true"));
        let finding_id = finding
            .attribute("id")
            .expect("the finding carries its stable id");
        assert!(finding_id.starts_with(&format!(
            "{}-",
            rendered
                .find(|tag| tag.element == "form")
                .and_then(|form| form.attribute("id").map(str::to_owned))
                .expect("the form carries its id")
        )));

        let button = tags(&summary)
            .into_iter()
            .find(|tag| tag.element == "button")
            .expect("the finding should be a button that focuses its target");
        assert_eq!(button.attribute("type"), Some("button"));
        assert!(button.has_classes(&["link", "text-start"]), "{button:?}");
        assert!(
            summary.contains("Value does not satisfy minLength."),
            "the finding text should be the localized finding: {summary}"
        );
    }
}
