//! The daisyUI constant control.

use dioxus::prelude::*;
use schemaform_dioxus::ControlRenderContext;

use super::parts::{display_text, read_only_field};

/// One daisyUI constant control: the renderer's hook-safe child component.
///
/// A constant has no edit hook and is never an editable widget. It is read-only output of the
/// presentation and facets: the write-only status where the value must not be shown, else the
/// fixed value's display text, with the presence affordances that materialize or remove it
/// unless the node itself is read-only.
#[component]
pub(super) fn ConstantControl(context: ControlRenderContext) -> Element {
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let presentation = context.presentation();
    let control = context.control();
    if projection.read_only {
        return read_only_field(presentation, control, display_text(&projection), &[]);
    }
    let text = control
        .write_only_status
        .clone()
        .unwrap_or_else(|| display_text(&projection));
    read_only_field(presentation, control, text, &presentation.presence)
}
