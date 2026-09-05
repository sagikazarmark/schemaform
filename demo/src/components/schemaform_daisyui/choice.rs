//! The daisyUI choice controls: a native select by default, a radio group or a compound select
//! when the UI schema asks for one.

use dioxus::prelude::*;
use schemaform_dioxus::{ChoiceIdentity, ControlRenderContext, use_choice_edit};

use super::Appearance;
use super::mapping::{use_choice_binding, use_radio_binding};
use super::parts::{WidgetLayout, editable, editable_field, kind_name};
use crate::components::native_select::{NativeSelect, NativeSelectOption};
use crate::components::radio_group::{RadioGroup, RadioItem};
use crate::components::select::{Select, SelectList, SelectOption, SelectTrigger, SelectValue};

/// The placeholder a choice widget shows while no option is selected.
///
/// A write-only control shows its replacement placeholder, since the value must not be echoed;
/// otherwise nothing, as the built-in's hidden placeholder does. Incompatible data is not shown
/// here but in the readout under the widget, which carries a stable id and describes the widget,
/// as for every other control kind.
fn placeholder(context: &ControlRenderContext) -> String {
    context
        .control()
        .write_only_replacement
        .as_ref()
        .map(|replacement| replacement.placeholder.clone())
        .unwrap_or_default()
}

/// One daisyUI choice control as a native select: the renderer's hook-safe child component.
///
/// Every option, the null option included, is a native `option` whose form value is the
/// option's opaque identity, so the select speaks the same strings the edit hook resynchronises
/// after a rejected write. The placeholder is a disabled first option selected while nothing is,
/// which is also where a write-only control rests after every write.
#[component]
pub(super) fn NativeSelectControl(
    context: ControlRenderContext,
    appearance: Appearance,
) -> Element {
    let edit = use_choice_edit(&context);
    let binding = use_choice_binding(edit.clone());
    if let Err(rendered) = editable(&context, appearance) {
        return rendered;
    }
    let control = context.control();
    let kind = kind_name(control.kind);
    let placeholder = placeholder(&context);
    let options = edit
        .options
        .iter()
        .map(|option| {
            NativeSelectOption::new(option.identity.clone(), option.label.clone())
                .form_value(option.identity.as_str())
                .disabled(option.disabled)
        })
        .collect::<Vec<_>>();

    editable_field(
        &context,
        appearance,
        binding,
        WidgetLayout::Stacked,
        rsx! {
            NativeSelect::<ChoiceIdentity> {
                options,
                placeholder,
                "data-schemaform-control": kind,
                "data-write-only-replacement": control.write_only.then_some(""),
            }
        },
    )
}

/// One daisyUI choice control as a radio group: the renderer's hook-safe child component,
/// selected by the `daisyui:radio` widget symbol.
///
/// Every option, the null option included, is a `RadioItem` whose value is the option's opaque
/// identity string, labelled by the text beside it. The group root carries the control's
/// element id and ARIA state; the hidden form participants the registry renders carry its name.
/// Nothing is checked while nothing is selected, which is also a write-only control's resting
/// state after every write. The registry reports focus exit one task after the group loses
/// focus, so touched state lands one task later than with the native select.
#[component]
pub(super) fn RadioGroupControl(context: ControlRenderContext, appearance: Appearance) -> Element {
    let edit = use_choice_edit(&context);
    let binding = use_radio_binding(edit.clone());
    if let Err(rendered) = editable(&context, appearance) {
        return rendered;
    }
    let control = context.control();
    let kind = kind_name(control.kind);
    let element_id = context.presentation().element_id.clone();
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

    let row_class = appearance.utilities("flex items-center gap-2");
    let item_label_class = appearance.utilities("text-sm");
    editable_field(
        &context,
        appearance,
        binding,
        WidgetLayout::Stacked,
        rsx! {
            RadioGroup {
                "data-schemaform-control": kind,
                "data-write-only-replacement": control.write_only.then_some(""),
                for (index , (option , item_id , item_label_id)) in items.into_iter().enumerate() {
                    div { key: "{item_id}", class: row_class,
                        RadioItem {
                            id: item_id.clone(),
                            value: option.identity.as_str().to_owned(),
                            index,
                            disabled: option.disabled,
                            aria_labelledby: item_label_id.clone(),
                        }
                        span { id: item_label_id, class: item_label_class, "{option.label}" }
                    }
                }
            }
        },
    )
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
pub(super) fn SelectControl(context: ControlRenderContext, appearance: Appearance) -> Element {
    let edit = use_choice_edit(&context);
    let binding = use_choice_binding(edit.clone());
    if let Err(rendered) = editable(&context, appearance) {
        return rendered;
    }
    let control = context.control();
    let kind = kind_name(control.kind);
    let placeholder = placeholder(&context);
    let options = edit.options.clone();

    editable_field(
        &context,
        appearance,
        binding,
        WidgetLayout::Stacked,
        rsx! {
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
        },
    )
}
