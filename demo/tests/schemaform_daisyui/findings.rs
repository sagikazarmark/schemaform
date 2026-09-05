//! The daisyUI finding presenter: the form-wide summary alert and node-local findings.

use schemaform::{ExternalFinding, ExternalFindingBatch, JsonPointer};
use serde_json::json;

use crate::support::{RenderedForm, arrays_app, inner_html, tags};

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

/// Applies one advisory external finding at `pointer` for the current data revision.
fn apply_advisory(rendered: &mut RenderedForm, pointer: &str, code: &str) {
    let revision = rendered
        .handle
        .reader()
        .read()
        .expect("the form should be readable")
        .data_revision;
    let handle = rendered.handle.clone();
    rendered.drive(|| {
        handle
            .apply_external_findings(ExternalFindingBatch::new(
                "server",
                revision,
                vec![ExternalFinding::advisory(
                    code,
                    JsonPointer::parse(pointer).expect("the pointer should parse"),
                    json!({}),
                )],
            ))
            .expect("the advisory batch should apply");
    });
}

/// When nothing blocks submission the summary takes the warning tone: an advisory finding is
/// still listed, with a focus button, but framed as `alert-warning` rather than `alert-error`.
#[test]
fn a_summary_of_advisory_findings_only_is_a_warning_alert() {
    let mut rendered = mount();
    let name = rendered.actions_at("/name");
    name.blur().expect("leaving the control should apply");
    apply_advisory(&mut rendered, "/name", "suggest-name");

    let summary = summary_html(&rendered);
    let alert = tags(&summary)
        .into_iter()
        .find(|tag| tag.classes().contains(&"alert"))
        .unwrap_or_else(|| panic!("the summary should be an alert:\n{summary}"));
    assert!(
        alert.has_classes(&["alert", "alert-warning", "alert-soft"]),
        "{alert:?}"
    );
    assert!(
        !alert.classes().contains(&"alert-error"),
        "nothing blocks: {alert:?}"
    );
    let finding = tags(&summary)
        .into_iter()
        .find(|tag| tag.attribute("data-finding").is_some())
        .expect("the advisory finding is listed");
    assert_eq!(finding.attribute("data-blocking"), Some("false"));
    assert!(
        tags(&summary).iter().any(|tag| tag.element == "button"),
        "an advisory finding still focuses its target: {summary}"
    );
}

/// A node-local advisory finding is a description in the warning colour, carrying its stable id
/// and code, and it describes its node without marking the node invalid.
#[test]
fn a_local_advisory_finding_is_a_warning_description_that_does_not_invalidate_its_node() {
    let mut rendered = mount();
    // A submission attempt makes untouched nodes' findings visible under the default policy.
    let handle = rendered.handle.clone();
    rendered.drive(|| {
        handle
            .prepare_submission()
            .expect("the submission attempt is prepared");
    });
    apply_advisory(&mut rendered, "/tags", "review-tags");

    let html = rendered.html();
    let tags_root = rendered
        .find(|tag| tag.attribute("data-schemaform-daisyui") == Some("collection"))
        .expect("the tags fieldset should be rendered");
    let tags_id = tags_root
        .attribute("id")
        .expect("the fieldset carries its id");
    assert_eq!(tags_root.attribute("aria-invalid"), Some("false"));
    let finding = tags(&inner_html(&html, tags_id))
        .into_iter()
        .find(|tag| tag.attribute("data-finding") == Some("review-tags"))
        .unwrap_or_else(|| panic!("the advisory finding renders in the fieldset: {html}"));
    assert_eq!(finding.element, "p");
    assert!(finding.has_classes(&["text-warning"]), "{finding:?}");
    assert!(!finding.classes().contains(&"text-error"), "{finding:?}");
    assert_eq!(finding.attribute("data-blocking"), Some("false"));
    let finding_id = finding.attribute("id").expect("stable id");
    assert!(
        tags_root
            .attribute("aria-describedby")
            .is_some_and(|value| value.split_whitespace().any(|id| id == finding_id)),
        "the finding describes the fieldset: {tags_root:?}"
    );
}
