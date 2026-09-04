//! The daisyUI boolean control.

use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_field::{
    ChangeOrigin, FieldContext, FieldControlOptions, FieldSurface, merge_attributes, use_binding,
    use_field_meta, use_focus_registration,
};
use dioxus_primitives::dioxus_attributes::attributes;
use schemaform_dioxus::{ControlRenderContext, use_boolean_edit};

use super::mapping::{field_meta_values, use_boolean_binding, use_checkbox_binding};
use super::parts::{
    display_text, incompatible_description, kind_name, label_class, label_id, read_only_field,
    supplements,
};
use crate::components::checkbox::Checkbox;
use crate::components::field::{Field, FieldLabel, FieldRow};
use crate::components::native_select::{NativeSelect, NativeSelectOption};

/// One daisyUI boolean control: the renderer's hook-safe child component.
///
/// A write-only boolean is a replacement select over the localized false and true labels, so the
/// value it holds is never shown; the tri-state binding never reads a value for it either. A
/// nullable boolean is the registry's `Checkbox` bound to the tri-state view of the edit, so
/// JSON null shows as the indeterminate state and the set-null affordance is how it is reached;
/// a click from either boolean state makes it a boolean again, as the widget defines toggling.
/// A non-nullable boolean is a native checkbox carrying daisyUI's `checkbox` class, which keeps
/// the built-in's native semantics; its two-state view of the edit is the tri-state binding with
/// null read as unchecked, which such a node never holds.
#[component]
pub(super) fn BooleanControl(context: ControlRenderContext) -> Element {
    let edit = use_boolean_edit(&context);
    let binding = use_boolean_binding(edit);
    let checkbox = use_checkbox_binding(edit);
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let presentation = context.presentation();
    let control = context.control();
    if projection.read_only {
        return read_only_field(presentation, control, display_text(&projection), &[]);
    }

    let kind = kind_name(control.kind);
    let label = presentation.label.clone();
    let meta = field_meta_values(presentation, control);

    if let (Some(replacement), Some(labels)) = (
        control.write_only_replacement.clone(),
        control.boolean_labels.clone(),
    ) {
        // The value must not be echoed, so the widget is a replacement select that the edit hook
        // puts back on its placeholder after every write, as the built-in does.
        let field_context = FieldContext::new(binding).with_meta_values(meta);
        let options = vec![
            NativeSelectOption::new(false, labels.false_label).form_value("false"),
            NativeSelectOption::new(true, labels.true_label).form_value("true"),
        ];
        return rsx! {
            Field { context: field_context, "data-schemaform-daisyui": kind,
                FieldLabel {
                    id: label_id(presentation),
                    class: label_class(presentation),
                    "{replacement.label}"
                }
                NativeSelect::<bool> {
                    options,
                    placeholder: replacement.placeholder,
                    "data-schemaform-control": kind,
                    "data-write-only-replacement": "",
                }
                {supplements(presentation)}
            }
        };
    }

    let incompatible = incompatible_description(presentation, &projection);
    if control.nullable {
        let field_context = FieldContext::new(checkbox).with_meta_values(meta);
        return rsx! {
            Field { context: field_context, "data-schemaform-daisyui": kind,
                FieldRow {
                    Checkbox { "data-schemaform-control": kind }
                    FieldLabel { id: label_id(presentation), class: label_class(presentation), "{label}" }
                }
                {incompatible}
                {supplements(presentation)}
            }
        };
    }

    let field_context = FieldContext::new(binding).with_meta_values(meta);
    rsx! {
        Field { context: field_context, "data-schemaform-daisyui": kind,
            FieldRow {
                NativeCheckbox { "data-schemaform-control": kind }
                FieldLabel { id: label_id(presentation), class: label_class(presentation), "{label}" }
            }
            {incompatible}
            {supplements(presentation)}
        }
    }
}

/// A native checkbox styled with daisyUI's `checkbox` class and bound the way the registry's
/// widgets are: it resolves the tri-state binding and the metadata from the surrounding `Field`,
/// reads and writes the binding's two-state view, and registers for the field's focus requests.
///
/// The element carries the adapter's element id through the metadata, which is what the edit
/// hook resynchronises after a rejected write. Required is spelled as `aria-required`, as the
/// built-in does: a native `required` on a checkbox would demand the checked state rather than
/// a present value. Classes passed by the caller concatenate with the checkbox's own; every
/// other attribute the caller passes overrides the checkbox's.
#[component]
fn NativeCheckbox(#[props(extends = GlobalAttributes)] attributes: Vec<Attribute>) -> Element {
    let binding = use_binding::<Option<bool>>(None, None);
    let meta = use_field_meta(None);
    let mut control: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let focus_control = use_callback(move |()| {
        if let Some(control) = control() {
            spawn(async move {
                let _ = control.set_focus(true).await;
            });
        }
    });
    use_focus_registration(focus_control);

    let checked = (binding.read)().unwrap_or(false);
    let color = if meta.invalid() { "checkbox-error" } else { "" };
    let base = attributes!(input {
        class: "checkbox {color}",
        r#type: "checkbox",
    });
    let meta_attributes =
        meta.attributes_for(&FieldControlOptions::new().surface(FieldSurface::BUTTON_WIDGET));
    let merged = merge_attributes(vec![meta_attributes, base, attributes]);
    let write_binding = binding.clone();
    let focus_exit_binding = binding;

    rsx! {
        input {
            checked,
            onmounted: move |event: MountedEvent| control.set(Some(event.data())),
            oninput: move |event: FormEvent| {
                write_binding.write(Some(event.checked()), ChangeOrigin::User);
            },
            onfocusout: move |_| focus_exit_binding.focus_exit(),
            ..merged,
        }
    }
}
