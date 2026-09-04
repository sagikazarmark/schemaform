//! The daisyUI choice controls: a native select by default, a radio group or a compound select
//! when the UI schema asks for one.

use dioxus::prelude::*;
use dioxus_field::FieldContext;
use schemaform::form::ScalarValueState;
use schemaform_dioxus::{ChoiceIdentity, ControlRenderContext, NodeProjection, use_choice_edit};

use super::mapping::{field_meta_values, use_choice_binding, use_radio_binding};
use super::parts::{
    display_text, incompatible_description, kind_name, label_class, label_id, read_only_field,
    supplements, widget_label,
};
use crate::components::field::{Field, FieldLabel};
use crate::components::native_select::{NativeSelect, NativeSelectOption};
use crate::components::radio_group::{RadioGroup, RadioItem};
use crate::components::select::{Select, SelectList, SelectOption, SelectTrigger, SelectValue};

/// The placeholder a choice widget shows while no option is selected.
///
/// A write-only control shows its replacement placeholder, since the value must not be echoed;
/// incompatible data shows the data itself, so the user sees what the replacement discards; a
/// missing value shows nothing, as the built-in's hidden placeholder does.
fn placeholder(context: &ControlRenderContext, projection: &NodeProjection) -> String {
    match &context.control().write_only_replacement {
        Some(replacement) => replacement.placeholder.clone(),
        None if matches!(projection.value_state, Some(ScalarValueState::Incompatible)) => {
            display_text(projection)
        }
        None => String::new(),
    }
}

/// One daisyUI choice control as a native select: the renderer's hook-safe child component.
///
/// Every option, the null option included, is a native `option` whose form value is the
/// option's opaque identity, so the select speaks the same strings the edit hook resynchronises
/// after a rejected write. The placeholder is a disabled first option selected while nothing is,
/// which is also where a write-only control rests after every write.
#[component]
pub(super) fn NativeSelectControl(context: ControlRenderContext) -> Element {
    let edit = use_choice_edit(&context);
    let binding = use_choice_binding(edit.clone());
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let presentation = context.presentation();
    let control = context.control();
    if projection.read_only {
        return read_only_field(presentation, control, display_text(&projection), &[]);
    }

    let kind = kind_name(control.kind);
    let label = widget_label(presentation, control);
    let placeholder = placeholder(&context, &projection);
    let options = edit
        .options
        .iter()
        .map(|option| {
            NativeSelectOption::new(option.identity.clone(), option.label.clone())
                .form_value(option.identity.as_str())
                .disabled(option.disabled)
        })
        .collect::<Vec<_>>();
    let field_context =
        FieldContext::new(binding).with_meta_values(field_meta_values(presentation, control));

    rsx! {
        Field { context: field_context, "data-schemaform-daisyui": kind,
            FieldLabel { id: label_id(presentation), class: label_class(presentation), "{label}" }
            NativeSelect::<ChoiceIdentity> {
                options,
                placeholder,
                "data-schemaform-control": kind,
                "data-write-only-replacement": control.write_only.then_some(""),
            }
            {supplements(presentation)}
        }
    }
}

/// One daisyUI choice control as a radio group: the renderer's hook-safe child component,
/// selected by the `daisyui:radio` widget symbol.
///
/// Every option, the null option included, is a `RadioItem` whose value is the option's opaque
/// identity string, labelled by the text beside it. The group root carries the control's
/// element id and ARIA state; the hidden form participants the registry renders carry its name.
/// Nothing is checked while nothing is selected, which is also a write-only control's resting
/// state after every write.
#[component]
pub(super) fn RadioGroupControl(context: ControlRenderContext) -> Element {
    let edit = use_choice_edit(&context);
    let binding = use_radio_binding(edit.clone());
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let presentation = context.presentation();
    let control = context.control();
    if projection.read_only {
        return read_only_field(presentation, control, display_text(&projection), &[]);
    }

    let kind = kind_name(control.kind);
    let label = widget_label(presentation, control);
    let element_id = presentation.element_id.clone();
    let incompatible = incompatible_description(presentation, &projection);
    // Each item and its label carry ids derived from the control's element id and the option's
    // identity, so they are stable per option and unique within the form.
    let items = edit
        .options
        .iter()
        .map(|option| {
            let item_id = format!("{element_id}-{}", option.identity.as_str());
            let label_id = format!("{item_id}-label");
            (option.clone(), item_id, label_id)
        })
        .collect::<Vec<_>>();
    let field_context =
        FieldContext::new(binding).with_meta_values(field_meta_values(presentation, control));

    rsx! {
        Field { context: field_context, "data-schemaform-daisyui": kind,
            FieldLabel { id: label_id(presentation), class: label_class(presentation), "{label}" }
            RadioGroup {
                "data-schemaform-control": kind,
                "data-write-only-replacement": control.write_only.then_some(""),
                for (index , (option , item_id , item_label_id)) in items.into_iter().enumerate() {
                    div { key: "{item_id}", class: "flex items-center gap-2",
                        RadioItem {
                            id: item_id.clone(),
                            value: option.identity.as_str().to_owned(),
                            index,
                            disabled: option.disabled,
                            aria_labelledby: item_label_id.clone(),
                        }
                        span { id: item_label_id, class: "text-sm", "{option.label}" }
                    }
                }
            }
            {incompatible}
            {supplements(presentation)}
        }
    }
}

/// One daisyUI choice control as the registry's compound select: the renderer's hook-safe child
/// component, selected by the `daisyui:select` widget symbol.
///
/// Every option, the null option included, is a `SelectOption` over the option's opaque identity
/// whose text is the localized label; the trigger shows that text for the selected option and the
/// placeholder otherwise. The trigger carries the control's element id and name. The registry
/// reports focus exit one task after the trigger loses focus, so touched state lands one task
/// later than with the native select.
#[component]
pub(super) fn SelectControl(context: ControlRenderContext) -> Element {
    let edit = use_choice_edit(&context);
    let binding = use_choice_binding(edit.clone());
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let presentation = context.presentation();
    let control = context.control();
    if projection.read_only {
        return read_only_field(presentation, control, display_text(&projection), &[]);
    }

    let kind = kind_name(control.kind);
    let label = widget_label(presentation, control);
    let placeholder = placeholder(&context, &projection);
    let options = edit.options.clone();
    let field_context =
        FieldContext::new(binding).with_meta_values(field_meta_values(presentation, control));

    rsx! {
        Field { context: field_context, "data-schemaform-daisyui": kind,
            FieldLabel { id: label_id(presentation), class: label_class(presentation), "{label}" }
            Select::<ChoiceIdentity> {
                SelectTrigger {
                    "data-schemaform-control": kind,
                    "data-write-only-replacement": control.write_only.then_some(""),
                    SelectValue { placeholder }
                }
                SelectList {
                    for (index , option) in options.into_iter().enumerate() {
                        SelectOption::<ChoiceIdentity> {
                            key: "{option.identity.as_str()}",
                            value: option.identity.clone(),
                            index,
                            text_value: Some(option.label.clone()),
                            disabled: option.disabled,
                            "{option.label}"
                        }
                    }
                }
            }
            {supplements(presentation)}
        }
    }
}
