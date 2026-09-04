//! Presentation parts shared by every daisyUI control kind: the read-only field, the
//! descriptions and errors under a widget, and the presence affordances.

use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_field::FieldContext;
use schemaform::form::ScalarValueState;
use schemaform_dioxus::{
    ControlFacets, ControlKind, NodePresentation, NodeProjection,
    render::{Affordance, FindingDescriptor, Help},
};

use super::mapping::{field_meta_values, is_field_error};
use crate::components::button::{Button, ButtonSize};
use crate::components::field::{Field, FieldDescription, FieldError, FieldLabel};

/// The kind's name for the `data-schemaform-control` marker the built-in also emits.
pub(super) fn kind_name(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::String => "string",
        ControlKind::Number => "number",
        ControlKind::Integer => "integer",
        ControlKind::Boolean => "boolean",
        ControlKind::Choice => "choice",
        ControlKind::Constant => "constant",
        // `ControlKind` is non-exhaustive. The renderer hands a kind this component does not know
        // to the built-in, so this arm is never reached.
        _ => "unknown",
    }
}

/// The class that keeps a label accessible when the presentation hides it.
pub(super) fn label_class(presentation: &NodePresentation) -> &'static str {
    if presentation.label_visible {
        ""
    } else {
        "sr-only"
    }
}

/// The label's own id: derived from the element id so it is stable and unique per control.
pub(super) fn label_id(presentation: &NodePresentation) -> String {
    format!("{}-label", presentation.element_id)
}

/// The label an editable widget shows: the replacement action for a write-only control, as the
/// built-in does, because the value it holds must not be described.
pub(super) fn widget_label(presentation: &NodePresentation, control: &ControlFacets) -> String {
    control
        .write_only_replacement
        .as_ref()
        .map(|replacement| replacement.label.clone())
        .unwrap_or_else(|| presentation.label.clone())
}

/// The display text of a node that is not being edited: the retained edit buffer or canonical
/// text, else the current data spelled as JSON, and nothing for a write-only value.
pub(super) fn display_text(projection: &NodeProjection) -> String {
    if projection.write_only && projection.edit_buffer.is_none() {
        return String::new();
    }
    projection.value.clone().unwrap_or_else(|| {
        projection
            .current_data
            .as_ref()
            .map(serde_json::Value::to_string)
            .unwrap_or_default()
    })
}

/// A description part per finding, each carrying the finding's stable id so every id the
/// adapter hands out resolves to an element.
fn finding_descriptions(findings: Vec<FindingDescriptor>) -> Element {
    rsx! {
        for finding in findings {
            FieldDescription {
                key: "{finding.stable_id}",
                id: Rc::from(finding.stable_id.as_str()),
                class: if finding.blocking { "text-error" } else { "text-warning" },
                "{finding.text}"
            }
        }
    }
}

/// The parts under an editable widget: help, the findings `FieldError` does not present, the
/// error region, and the presence affordances.
///
/// `FieldError` presents the field errors from the metadata; the remaining findings are
/// presented as further descriptions so every stable id still resolves to an element.
pub(super) fn supplements(presentation: &NodePresentation) -> Element {
    let help = presentation.help.clone();
    let descriptions = presentation
        .findings
        .iter()
        .filter(|finding| !is_field_error(finding))
        .cloned()
        .collect::<Vec<_>>();
    let errors_id = format!("{}-errors", presentation.element_id);
    rsx! {
        {help_description(help)}
        {finding_descriptions(descriptions)}
        FieldError { id: Rc::from(errors_id.as_str()) }
        {presence_affordances(&presentation.presence)}
    }
}

/// The help text as a description part carrying the adapter's help id.
fn help_description(help: Option<Help>) -> Element {
    rsx! {
        if let Some(help) = help {
            FieldDescription { id: Rc::from(help.id.as_str()), "{help.text}" }
        }
    }
}

/// The current data shown beside a widget that cannot show it itself, as the built-in shows it
/// beside its checkbox: present while the value is incompatible, or null where null is not
/// accepted, the core allows replacement, and the control is not write-only. The description
/// carries a stable id and describes the widget, so the replace affordance beside it has context.
pub(super) fn incompatible_description(
    presentation: &NodePresentation,
    projection: &NodeProjection,
) -> Element {
    let operations = projection.allowed_operations;
    let incompatible = (matches!(
        projection.value_state,
        Some(ScalarValueState::Incompatible | ScalarValueState::Null)
            if !operations.can_input_text() && operations.can_replace_value()
    ) && !projection.write_only)
        .then(|| {
            projection
                .current_data
                .as_ref()
                .map(serde_json::Value::to_string)
                .unwrap_or_default()
        });
    let id = format!("{}-incompatible", presentation.element_id);
    rsx! {
        if let Some(value) = incompatible {
            FieldDescription {
                id: Rc::from(id.as_str()),
                class: "text-warning",
                "data-incompatible-value": "",
                "{value}"
            }
        }
    }
}

/// The presence affordances, rendered as daisyUI buttons carrying the ids the adapter expects.
///
/// Presence affordances carry no accessible name today; should one arrive, it names the button
/// as the adapter intends.
pub(super) fn presence_affordances(presence: &[Affordance]) -> Element {
    let presence = presence.to_vec();
    rsx! {
        if !presence.is_empty() {
            div { class: "flex flex-wrap gap-2",
                for affordance in presence {
                    Button {
                        key: "{affordance.id}",
                        id: affordance.id.clone(),
                        r#type: "button",
                        size: ButtonSize::Sm,
                        "aria-label": affordance.accessible_name.clone(),
                        onclick: move |_| affordance.invoke.call(()),
                        "{affordance.label}"
                    }
                }
            }
        }
    }
}

/// A decorative Heroicons outline icon: `path` is the icon's path data, `class` sizes it. Hidden
/// from assistive technology, so the element it decorates must carry its own name.
pub(super) fn icon(path: &'static str, class: &'static str) -> Element {
    rsx! {
        svg {
            class,
            "aria-hidden": "true",
            xmlns: "http://www.w3.org/2000/svg",
            fill: "none",
            view_box: "0 0 24 24",
            stroke_width: "1.5",
            stroke: "currentColor",
            path { stroke_linecap: "round", stroke_linejoin: "round", d: path }
        }
    }
}

/// A node nothing edits: label, noninteractive `output`, every finding as a description of the
/// shown value, and the `presence` affordances that can still repair it.
///
/// The field context carries metadata only; no widget resolves a binding from it, and the output
/// carries the adapter's own `aria-describedby`. A read-only node passes no affordances, as the
/// built-in offers none for it; a constant passes its own, which materialize or remove the fixed
/// value.
pub(super) fn read_only_field(
    presentation: &NodePresentation,
    control: &ControlFacets,
    text: String,
    presence: &[Affordance],
) -> Element {
    let field_context =
        FieldContext::empty().with_meta_values(field_meta_values(presentation, control));
    let kind = kind_name(control.kind);
    let element_id = presentation.element_id.clone();
    let label = presentation.label.clone();
    let help = presentation.help.clone();
    let findings = presentation.findings.clone();
    rsx! {
        Field { context: field_context, "data-schemaform-daisyui": kind,
            FieldLabel { id: label_id(presentation), class: label_class(presentation), "{label}" }
            output {
                id: element_id,
                name: control.name.clone(),
                class: "min-w-0 py-2",
                tabindex: "-1",
                "data-read-only": "",
                "data-schemaform-control": kind,
                "aria-invalid": presentation.invalid,
                "aria-describedby": presentation.described_by(),
                "{text}"
            }
            {help_description(help)}
            {finding_descriptions(findings)}
            {presence_affordances(presence)}
        }
    }
}
