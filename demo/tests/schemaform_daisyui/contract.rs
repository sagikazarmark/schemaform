//! The adapter's contract, exercised through the daisyUI package as a real consumer.
//!
//! These tests stay with schemaform when the component moves to its registry. They fail when
//! `schemaform-dioxus` breaks a seam — binding, the ids it hands out, finding visibility, the
//! summary — not when daisyUI changes its markup, so they assert on bindings, ARIA
//! relationships, and the adapter's own markers rather than on classes or widget structure.

use dioxus::prelude::*;
use schemaform::{ExternalFinding, ExternalFindingBatch, FormDefinition, JsonPointer};
use schemaform_dioxus::use_form;
use serde_json::json;

use crate::support::{
    RenderedForm, TestAppProps, assert_aria_references_resolve, gallery_app, inner_html, tags,
};
use demo::components::schemaform_daisyui::SchemaformDaisyui;

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

/// Every id the adapter hands out resolves to an element once every finding shape is visible at
/// once — a parse blocker, a validation finding, a blocking and an advisory external finding,
/// and help — whether the renderer presented it as a description or in an error region.
#[test]
fn every_id_the_adapter_hands_out_resolves_with_every_finding_shape_visible() {
    let mut rendered = RenderedForm::mount(gallery_app);
    let quantity = rendered.actions_at("/quantity");
    let name = rendered.actions_at("/name");
    let price = rendered.actions_at("/price");
    rendered.drive(|| {
        quantity
            .input_text("-")
            .expect("the parse blocker should be recorded");
        name.input_text("A").expect("the edit should apply");
        // Findings on an untouched control are not visible under the default policy.
        name.blur().expect("leaving the control should apply");
        price.blur().expect("leaving the control should apply");
    });
    // External findings are scoped to a data revision, so the batch names the one the edits
    // above produced.
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
                vec![
                    ExternalFinding::blocking(
                        "review-name",
                        JsonPointer::parse("/name").expect("pointer"),
                        json!({}),
                    ),
                    ExternalFinding::advisory(
                        "suggest-price",
                        JsonPointer::parse("/price").expect("pointer"),
                        json!({}),
                    ),
                ],
            ))
            .expect("the external batch should apply");
    });

    let html = rendered.html();
    for (pointer, invalid) in [("/quantity", true), ("/name", true), ("/price", false)] {
        let control = rendered.control(pointer);
        assert_eq!(
            control.attribute("aria-invalid"),
            Some(if invalid { "true" } else { "false" }),
            "{pointer}: {control:?}"
        );
        assert_eq!(
            control.attribute("aria-errormessage").is_some(),
            invalid,
            "only an invalid control references an error element: {control:?}"
        );
    }
    let quantity_errors = rendered
        .control("/quantity")
        .attribute("aria-errormessage")
        .expect("checked above")
        .to_owned();
    assert!(
        tags(&inner_html(&html, &quantity_errors))
            .iter()
            .any(|tag| tag.attribute("data-blocking") == Some("true")),
        "the referenced error element holds the blocking finding: {html}"
    );
    assert!(
        rendered
            .control("/price")
            .attribute("aria-describedby")
            .is_some_and(
                |value| value
                    .split_whitespace()
                    .any(|id| tags(&html).iter().any(|tag| {
                        tag.attribute("id") == Some(id)
                            && tag.attribute("data-blocking") == Some("false")
                    }))
            ),
        "the advisory finding describes its control: {html}"
    );
    assert!(assert_aria_references_resolve(&html) >= 8, "{html}");
}

/// A blocked submission attempt makes the untouched controls' findings visible everywhere at
/// once — on each control and in the summary, which lists one entry per blocking finding.
#[test]
fn a_blocked_submission_surfaces_its_findings_on_the_controls_and_in_the_summary() {
    let mut rendered = RenderedForm::mount(gallery_app);
    let quantity = rendered.actions_at("/quantity");
    let name = rendered.actions_at("/name");
    let handle = rendered.handle.clone();
    rendered.drive(|| {
        // Neither control is left, so nothing is visible before the attempt.
        quantity
            .input_text("-")
            .expect("the parse blocker should be recorded");
        name.input_text("A").expect("the edit should apply");
    });
    assert_eq!(
        rendered.control("/name").attribute("aria-invalid"),
        Some("false"),
        "an untouched control's validation finding is not visible before submission"
    );

    rendered.drive(|| {
        handle
            .prepare_submission()
            .expect("the submission attempt is prepared");
    });

    let html = rendered.html();
    assert_eq!(
        rendered.control("/name").attribute("aria-invalid"),
        Some("true")
    );
    assert_eq!(
        rendered.control("/quantity").attribute("aria-invalid"),
        Some("true")
    );
    let region = rendered
        .find(|tag| tag.attribute("data-finding-summary").is_some())
        .expect("the adapter's summary region is placed by the shell");
    let summary = tags(&inner_html(
        &html,
        region.attribute("id").expect("the region carries its id"),
    ));
    let listed = summary
        .iter()
        .filter(|tag| tag.attribute("data-finding").is_some())
        .count();
    assert_eq!(listed, 2, "one summary entry per blocking finding: {html}");
    assert!(
        summary.iter().filter(|tag| tag.element == "button").count() >= 2,
        "each entry focuses its target: {html}"
    );
}
