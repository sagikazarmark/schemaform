//! Presentation parts shared by every daisyUI control kind: the editable frame around a widget,
//! the read-only field, the descriptions and errors under a widget, and the presence affordances.

use std::{cell::RefCell, rc::Rc};

use dioxus::prelude::*;
use dioxus_field::{Binding, FieldContext, FieldMeta, FieldMetaIdRegistration, use_field_meta};
use schemaform_dioxus::{
    ControlFacets, ControlKind, ControlRenderContext, NodePresentation, NodeProjection,
    render::{Affordance, FindingDescriptor, Help},
};

use super::Appearance;
use super::mapping::{field_meta_values, is_field_error};
use crate::components::button::{Button, ButtonSize};
use crate::components::field::{Field, FieldDescription, FieldLabel, FieldRow};

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

/// The projection of a control something can edit, or the element to render instead: nothing
/// while the node is gone, the read-only field while nothing edits the node.
///
/// Call it after the component's hooks, so the hook order is the same on every render. The
/// read-only rule lives here once: a read-only node is noninteractive `output` of its display
/// text with no presence affordances, as the built-in offers none for it.
pub(super) fn editable(
    context: &ControlRenderContext,
    appearance: Appearance,
) -> Result<NodeProjection, Element> {
    let Some(projection) = context.node().read().ok().flatten() else {
        return Err(rsx! {});
    };
    if projection.read_only {
        return Err(read_only_field(
            context.presentation(),
            context.control(),
            appearance,
            projection.display_text(),
            &[],
        ));
    }
    Ok(projection)
}

/// Where an editable widget sits relative to its label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WidgetLayout {
    /// The label above the widget, as for an input or a select.
    Stacked,
    /// A checkable widget beside its label in a `FieldRow`.
    Row,
}

/// The frame every editable daisyUI control shares: the registry `Field` over `binding` carrying
/// the control's metadata, the label, the widget, the incompatible-value readout, and the
/// supplements under it.
///
/// The label is [`widget_label`]: the replacement action for a write-only control, else the
/// node's label. The readout is [`NodePresentation::incompatible_value`], so a widget that cannot
/// show the data it would replace (an input holding a number where the schema wants a string, a
/// checkbox holding a string) still tells the user what the replace affordance discards.
pub(super) fn editable_field<T: 'static>(
    context: &ControlRenderContext,
    appearance: Appearance,
    binding: Binding<T>,
    layout: WidgetLayout,
    widget: Element,
) -> Element {
    let presentation = context.presentation();
    let control = context.control();
    let field_context =
        FieldContext::new(binding).with_meta_values(field_meta_values(presentation, control));
    let kind = kind_name(control.kind);
    let label = widget_label(presentation, control);
    let label_id = label_id(presentation);
    let label_class = label_class(presentation);
    rsx! {
        Field { context: field_context, "data-schemaform-daisyui": kind,
            match layout {
                WidgetLayout::Stacked => rsx! {
                    FieldLabel { id: label_id, class: label_class, "{label}" }
                    {widget}
                },
                WidgetLayout::Row => rsx! {
                    FieldRow {
                        {widget}
                        FieldLabel { id: label_id, class: label_class, "{label}" }
                    }
                },
            }
            {incompatible_description(presentation, appearance)}
            {supplements(presentation, appearance)}
        }
    }
}

/// A description part per finding, each carrying the finding's stable id so every id the
/// adapter hands out resolves to an element, plus `data-finding` (the code) and `data-blocking`
/// as the finding presenter emits them.
fn finding_descriptions(findings: Vec<FindingDescriptor>, appearance: Appearance) -> Element {
    rsx! {
        for finding in findings {
            FieldDescription {
                key: "{finding.stable_id}",
                id: Rc::from(finding.stable_id.as_str()),
                class: appearance.utilities(if finding.blocking { "text-error" } else { "text-warning" }),
                "data-finding": finding.code.clone(),
                "data-blocking": finding.blocking.to_string(),
                "{finding.text}"
            }
        }
    }
}

/// The parts under an editable widget: help, the findings that are not field errors as
/// descriptions, the error region, and the presence affordances.
///
/// Findings [`is_field_error`] accepts are presented in the error region the control references
/// through `aria-errormessage`; every other finding is a description the control references
/// through `aria-describedby`. Both kinds of element carry the finding's stable id.
pub(super) fn supplements(presentation: &NodePresentation, appearance: Appearance) -> Element {
    let help = presentation.help.clone();
    let (errors, descriptions): (Vec<_>, Vec<_>) = presentation
        .findings
        .iter()
        .cloned()
        .partition(is_field_error);
    let errors_id = format!("{}-errors", presentation.element_id);
    rsx! {
        {help_description(help)}
        {finding_descriptions(descriptions, appearance)}
        FindingErrors { id: errors_id, findings: errors, appearance }
        {presence_affordances(&presentation.presence, appearance)}
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

/// The error region of a field: the element the control's `aria-errormessage` references and a
/// polite live region, as the registry's `FieldError` renders it, with one element per blocking
/// finding carrying the finding's stable id.
///
/// The registry's `FieldError` reads its text from the field metadata and renders it without
/// ids, so this part registers the same error id with the surrounding `Field`'s metadata and
/// renders the findings itself. It is always mounted so the live region exists before the first
/// error arrives; it is empty while nothing blocks.
#[component]
fn FindingErrors(id: String, findings: Vec<FindingDescriptor>, appearance: Appearance) -> Element {
    let meta = use_field_meta(None);
    let id: Rc<str> = Rc::from(id.as_str());
    use_error_id_registration(meta, Rc::clone(&id));
    rsx! {
        div {
            id: id.to_string(),
            class: appearance.utilities("text-error"),
            "aria-live": "polite",
            "data-schemaform-errors": "",
            for finding in findings {
                div {
                    key: "{finding.stable_id}",
                    id: finding.stable_id.clone(),
                    "data-finding": finding.code.clone(),
                    "data-blocking": "true",
                    "{finding.text}"
                }
            }
        }
    }
}

/// Keeps `id` registered as the error element of `meta` for as long as this component lives,
/// re-registering when either changes, as `dioxus-field`'s own parts do.
fn use_error_id_registration(meta: FieldMeta, id: Rc<str>) {
    let active = use_hook(|| Rc::new(RefCell::new(None::<ActiveErrorRegistration>)));
    let should_replace = active
        .borrow()
        .as_ref()
        .is_none_or(|active| active.meta != meta || active.id != id);
    if should_replace {
        let mut writable = meta;
        let registration = writable.register_error_id(Rc::clone(&id));
        active.borrow_mut().replace(ActiveErrorRegistration {
            meta,
            id,
            _registration: registration,
        });
    }
}

struct ActiveErrorRegistration {
    meta: FieldMeta,
    id: Rc<str>,
    _registration: FieldMetaIdRegistration,
}

/// The current data shown beside a widget that cannot show it itself, as the built-in shows it
/// beside its checkbox: [`NodePresentation::incompatible_value`], present while the value is
/// incompatible, or null where null is not accepted, the core allows replacement, and the
/// control is not write-only. The description carries a stable id and describes the widget, so
/// the replace affordance beside it has context.
pub(super) fn incompatible_description(
    presentation: &NodePresentation,
    appearance: Appearance,
) -> Element {
    let value = presentation.incompatible_value.clone();
    let id = format!("{}-incompatible", presentation.element_id);
    rsx! {
        if let Some(value) = value {
            FieldDescription {
                id: Rc::from(id.as_str()),
                class: appearance.utilities("text-warning"),
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
pub(super) fn presence_affordances(presence: &[Affordance], appearance: Appearance) -> Element {
    let presence = presence.to_vec();
    rsx! {
        if !presence.is_empty() {
            div { class: appearance.utilities("flex flex-wrap gap-2"),
                for affordance in presence {
                    {presence_button(affordance)}
                }
            }
        }
    }
}

/// One presence affordance as a small daisyUI button carrying its id.
fn presence_button(affordance: Affordance) -> Element {
    let invoke = affordance.clone();
    rsx! {
        Button {
            key: "{affordance.id}",
            id: affordance.id.clone(),
            r#type: "button",
            size: ButtonSize::Sm,
            "aria-label": affordance.accessible_name.clone(),
            onclick: move |_| invoke.invoke(),
            "{affordance.label}"
        }
    }
}

/// A decorative Heroicons outline icon: `path` is the icon's path data, `class` sizes it. Hidden
/// from assistive technology, so the element it decorates must carry its own name. The `width`
/// and `height` attributes give it an intrinsic `1em` size for when no class sizes it.
pub(super) fn icon(path: &'static str, class: &'static str) -> Element {
    rsx! {
        svg {
            class,
            width: "1em",
            height: "1em",
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
    appearance: Appearance,
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
                class: appearance.utilities("min-w-0 py-2"),
                tabindex: "-1",
                "data-read-only": "",
                "data-schemaform-control": kind,
                "aria-invalid": presentation.invalid,
                "aria-describedby": presentation.described_by(),
                "{text}"
            }
            {help_description(help)}
            {finding_descriptions(findings, appearance)}
            {presence_affordances(presence, appearance)}
        }
    }
}
