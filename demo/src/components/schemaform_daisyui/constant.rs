//! The daisyUI constant control.

use dioxus::prelude::*;
use schemaform_dioxus::ControlRenderContext;

use super::parts::{editable, read_only_field};

/// One daisyUI constant control: the renderer's hook-safe child component.
///
/// A constant has no edit hook and is never an editable widget. It is read-only output of the
/// presentation and facets: the write-only status where the value must not be shown, else the
/// fixed value's display text, with the presence affordances that materialize or remove it
/// unless the node itself is read-only.
#[component]
pub(super) fn ConstantControl(context: ControlRenderContext) -> Element {
    let projection = match editable(&context) {
        Ok(projection) => projection,
        Err(rendered) => return rendered,
    };
    let presentation = context.presentation();
    let control = context.control();
    let text = control
        .write_only_status
        .clone()
        .unwrap_or_else(|| projection.display_text());
    read_only_field(presentation, control, text, &presentation.presence)
}
