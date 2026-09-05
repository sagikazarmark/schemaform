//! The daisyUI text control: strings, numbers, and integers.

use dioxus::prelude::*;
use dioxus_primitives::dioxus_attributes::attributes;
use schemaform_dioxus::{ControlKind, ControlRenderContext, use_text_edit};

use super::Appearance;
use super::mapping::use_text_binding;
use super::parts::{WidgetLayout, editable, editable_field, kind_name};
use crate::components::input::Input;

/// The `inputmode` hint for a text control kind, as the built-in emits it.
fn input_mode(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::Number => "decimal",
        ControlKind::Integer => "numeric",
        _ => "text",
    }
}

/// One daisyUI text control: the renderer's hook-safe child component.
///
/// `Field` receives a fresh context on every render whose binding compares equal across renders
/// (its identity is the edit's hook-stable handles) and whose metadata values it syncs itself,
/// so the registry parts re-render only when the node's presentation actually changes. A
/// write-only control is a password input labelled with its replacement action, so the value it
/// holds is never shown.
///
/// A read-only node renders as noninteractive `output` rather than an `Input` that merely
/// rejects edits, as the built-in does; the facets' `read_only` also covers a node the core will
/// not accept text for right now, which keeps its editable widget and its replace affordance.
#[component]
pub(super) fn TextControl(context: ControlRenderContext, appearance: Appearance) -> Element {
    let edit = use_text_edit(&context);
    let binding = use_text_binding(edit);
    if let Err(rendered) = editable(&context, appearance) {
        return rendered;
    }
    let control = context.control();
    let kind = kind_name(control.kind);
    let placeholder = control
        .write_only_replacement
        .as_ref()
        .map(|replacement| replacement.placeholder.clone());

    // Listeners cannot travel through `extends`, so the composition events reach the native
    // input through the widget's explicit attribute list together with its other attributes.
    let input_attributes = attributes!(input {
        r#type: if control.write_only { "password" } else { "text" },
        inputmode: input_mode(control.kind),
        readonly: edit.read_only,
        placeholder,
        "data-schemaform-control": kind,
        "data-write-only-replacement": control.write_only.then_some(""),
        oncompositionstart: move |_| edit.composition_start.call(()),
        oncompositionend: move |_| edit.composition_end.call(()),
    });

    editable_field(
        &context,
        appearance,
        binding,
        WidgetLayout::Stacked,
        rsx! {
            Input { attributes: input_attributes }
        },
    )
}
