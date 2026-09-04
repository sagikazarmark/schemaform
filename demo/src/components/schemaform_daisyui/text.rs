//! The daisyUI string, number, and integer control.

use dioxus::prelude::*;
use dioxus_field::FieldContext;
use dioxus_primitives::dioxus_attributes::attributes;
use schemaform_dioxus::{ControlKind, ControlRenderContext, use_text_edit};

use super::mapping::{field_meta_values, use_text_binding};
use super::parts::{
    display_text, kind_name, label_class, label_id, read_only_field, supplements, widget_label,
};
use crate::components::field::{Field, FieldLabel};
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
/// so the registry parts re-render only when the node's presentation actually changes.
///
/// A read-only node renders as noninteractive `output` rather than an `Input` that merely
/// rejects edits, as the built-in does; the facets' `read_only` also covers a node the core will
/// not accept text for right now, which keeps its editable widget and its replace affordance.
#[component]
pub(super) fn TextControl(context: ControlRenderContext) -> Element {
    let edit = use_text_edit(&context);
    let binding = use_text_binding(edit);
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let presentation = context.presentation();
    let control = context.control();
    if projection.read_only {
        return read_only_field(presentation, control, display_text(&projection), &[]);
    }

    let field_context =
        FieldContext::new(binding).with_meta_values(field_meta_values(presentation, control));
    let kind = kind_name(control.kind);
    let label = widget_label(presentation, control);
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
        oncompositionstart: move |_| edit.composition_start.call(()),
        oncompositionend: move |_| edit.composition_end.call(()),
    });

    rsx! {
        Field { context: field_context, "data-schemaform-daisyui": kind,
            FieldLabel { id: label_id(presentation), class: label_class(presentation), "{label}" }
            Input { attributes: input_attributes }
            {supplements(presentation)}
        }
    }
}
