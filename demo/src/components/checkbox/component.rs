use dioxus::core::{AttributeValue, ListenerCallback};
use dioxus::prelude::*;
use dioxus_field::{
    Binding, ChangeOrigin, FieldContext, FieldControlOptions, FieldMeta, FieldSurface,
    merge_attributes, use_field_meta, use_focus_registration,
};
use dioxus_primitives::dioxus_attributes::attributes;
use std::rc::Rc;

use super::primitive;
use crate::components::field::{
    Field, FieldAppearance, FieldDescription, FieldDescriptionAppearance, FieldError,
    FieldErrorAppearance, FieldLabel, FieldRow, FieldRowAppearance,
};

pub use dioxus_primitives::checkbox::CheckboxState;

#[derive(Clone, PartialEq)]
enum ResolvedCheckboxBinding {
    State(Binding<CheckboxState>),
    Boolean(Binding<bool>),
}

impl ResolvedCheckboxBinding {
    fn read(&self) -> CheckboxState {
        match self {
            Self::State(binding) => (binding.read)(),
            Self::Boolean(binding) => {
                if (binding.read)() {
                    CheckboxState::Checked
                } else {
                    CheckboxState::Unchecked
                }
            }
        }
    }

    fn write(&self, value: CheckboxState, origin: ChangeOrigin) {
        match self {
            Self::State(binding) => binding.write(value, origin),
            Self::Boolean(binding) => {
                binding.write(value == CheckboxState::Checked, origin);
            }
        }
    }

    fn commit(&self) {
        match self {
            Self::State(binding) => binding.commit(),
            Self::Boolean(binding) => binding.commit(),
        }
    }

    fn focus_exit(&self) {
        match self {
            Self::State(binding) => binding.focus_exit(),
            Self::Boolean(binding) => binding.focus_exit(),
        }
    }
}

fn use_checkbox_binding(
    binding: Option<Binding<CheckboxState>>,
    bool_binding: Option<Binding<bool>>,
    default_value: CheckboxState,
) -> ResolvedCheckboxBinding {
    let internal: Binding<CheckboxState> = use_signal(|| default_value).into();

    if let Some(binding) = binding {
        return ResolvedCheckboxBinding::State(binding);
    }
    if let Some(binding) = bool_binding {
        return ResolvedCheckboxBinding::Boolean(binding);
    }

    let Some(context) = try_consume_context::<FieldContext>() else {
        return ResolvedCheckboxBinding::State(internal);
    };
    match context.try_resolve::<CheckboxState>() {
        Ok(Some(binding)) => ResolvedCheckboxBinding::State(binding),
        Ok(None) => ResolvedCheckboxBinding::State(internal),
        Err(state_mismatch) => match context.try_resolve::<bool>() {
            Ok(Some(binding)) => ResolvedCheckboxBinding::Boolean(binding),
            Ok(None) => unreachable!("the same Field Context contained a binding above"),
            Err(_) => panic!(
                "Field Context contains a binding for {}, but Checkbox supports bindings for {} or bool",
                state_mismatch.actual_type_name(),
                state_mismatch.requested_type_name(),
            ),
        },
    }
}

/// daisyUI's colour axis for a checkbox.
///
/// [`CheckboxColor::Default`] emits no class at all, which is daisyUI's own
/// uncoloured checkbox rather than a synonym for [`CheckboxColor::Neutral`].
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum CheckboxColor {
    #[default]
    Default,
    Neutral,
    Primary,
    Secondary,
    Accent,
    Info,
    Success,
    Warning,
    Error,
}

impl CheckboxColor {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[
        Self::Default,
        Self::Neutral,
        Self::Primary,
        Self::Secondary,
        Self::Accent,
        Self::Info,
        Self::Success,
        Self::Warning,
        Self::Error,
    ];

    /// The daisyUI class name for this value, as a complete string literal so
    /// Tailwind's scanner can see it.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::Neutral => "checkbox-neutral",
            Self::Primary => "checkbox-primary",
            Self::Secondary => "checkbox-secondary",
            Self::Accent => "checkbox-accent",
            Self::Info => "checkbox-info",
            Self::Success => "checkbox-success",
            Self::Warning => "checkbox-warning",
            Self::Error => "checkbox-error",
        }
    }
}

/// daisyUI's size axis for a checkbox.
///
/// [`CheckboxSize::Default`] emits no class, which renders at the same size as
/// daisyUI's explicit `checkbox-md`.
#[derive(Copy, Clone, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum CheckboxSize {
    Xs,
    Sm,
    #[default]
    Default,
    Lg,
    Xl,
}

impl CheckboxSize {
    /// Every value of this axis, in the order the preview renders them.
    pub const ALL: &'static [Self] = &[Self::Xs, Self::Sm, Self::Default, Self::Lg, Self::Xl];

    /// The daisyUI class name for this value, as a complete string literal so
    /// Tailwind's scanner can see it.
    pub const fn class(self) -> &'static str {
        match self {
            Self::Xs => "checkbox-xs",
            Self::Sm => "checkbox-sm",
            Self::Default => "",
            Self::Lg => "checkbox-lg",
            Self::Xl => "checkbox-xl",
        }
    }
}

/// A checkbox styled with daisyUI's `checkbox` classes.
///
/// The checked state needs no bridging: daisyUI matches the `aria-checked`
/// attribute the primitive already sets, so nothing here emits a class for it.
/// Producer-defined invalidity emits `checkbox-error` when no colour is passed.
/// A tri-state `binding` wins over a two-state `bool_binding`; otherwise either
/// type resolves from Field Context before standalone state. Metadata and focus
/// follow their ordinary explicit, context, then standalone precedence.
///
/// The primitive's indicator part is not exposed; daisyUI draws the mark
/// itself. A caller who wants a mark of their own uses the primitive directly.
///
/// Classes passed by the caller concatenate with the checkbox's own; every
/// other attribute the caller passes overrides the checkbox's.
///
/// Unlike the button, this takes no `extends = button` list. The primitive
/// renders the `button` element and owns the attributes that would be worth
/// reaching (including `type`, submitted value and `disabled`) as props of
/// its own, so
/// extending them here would offer a second, conflicting way to set them.
#[component]
pub fn Checkbox(
    /// An explicit colour, or `None` to derive error colour from Field metadata.
    #[props(default)]
    color: Option<CheckboxColor>,
    /// daisyUI's size axis.
    #[props(default)]
    size: CheckboxSize,
    /// An explicit Field binding, which wins over Field Context.
    binding: Option<Binding<CheckboxState>>,
    /// An explicit two-state Field binding, used when `binding` is absent.
    bool_binding: Option<Binding<bool>>,
    /// Explicit Field metadata, which wins over Field Context.
    meta: Option<FieldMeta>,
    /// The controlled value of the checkbox.
    #[props(default)]
    value: ReadSignal<Option<CheckboxState>>,
    /// The state the checkbox starts in when it is not controlled.
    #[props(default = CheckboxState::Unchecked)]
    default_value: CheckboxState,
    /// Whether the checkbox is required in a form.
    #[props(default)]
    required: Option<bool>,
    /// Whether the checkbox is disabled.
    #[props(default)]
    disabled: Option<bool>,
    /// The name of the checkbox, used in forms.
    #[props(default)]
    name: Option<String>,
    /// The submitted value of the checkbox. The default repeats the
    /// primitive's own, since a prop declared here has to carry one.
    #[props(default = ReadSignal::new(Signal::new(String::from("on"))))]
    form_value: ReadSignal<String>,
    /// Called with the checkbox's value after user interaction.
    on_change: Option<EventHandler<CheckboxState>>,
    /// Called after every change ends its interaction unit.
    on_commit: Option<EventHandler<()>>,
    /// Called after focus leaves the checkbox button.
    on_focus_exit: Option<EventHandler<()>>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
) -> Element {
    let binding = use_checkbox_binding(binding, bool_binding, default_value);
    let meta = use_field_meta(meta);
    let color = color.map_or_else(
        || {
            if meta.invalid() { "checkbox-error" } else { "" }
        },
        CheckboxColor::class,
    );
    let size = size.class();
    let binding_value = binding.clone();
    let checked = use_memo(move || {
        Some(match value() {
            Some(value) => value,
            None => binding_value.read(),
        })
    });
    let resolved_required = required.unwrap_or_else(|| meta.required());
    let resolved_disabled = disabled.unwrap_or_else(|| meta.disabled());
    let resolved_name = name
        .clone()
        .or_else(|| meta.name().map(|name| name.to_string()));
    let mut control: Signal<Option<Rc<MountedData>>> = use_signal(|| None);
    let focus_control = use_callback(move |()| {
        if let Some(control) = control() {
            spawn(async move {
                let _ = control.set_focus(true).await;
            });
        }
    });
    use_focus_registration(focus_control);

    // `button` rather than a checkbox element of some kind: the primitive
    // renders a `button` with `role="checkbox"`, and this list ends up spread
    // onto it, so that is the element the attribute has to be namespaced for.
    let base = attributes!(button {
        class: "checkbox {color} {size}",
    });
    let meta_attributes = meta.attributes_for(
        &FieldControlOptions::new()
            .disabled(disabled)
            .required(required)
            .name(name.map(Rc::from))
            .surface(FieldSurface::BUTTON_WIDGET),
    );
    let change_binding = binding.clone();
    let commit_binding = binding.clone();
    let focus_exit_binding = binding;
    let mut merged = merge_attributes(vec![meta_attributes, base, attributes]);
    let caller_focus_out = take_event_listener(&mut merged, "onfocusout");
    let interaction = attributes!(button {
        onmounted: move |event: MountedEvent| control.set(Some(event.data())),
        onfocusout: move |event: FocusEvent| {
            if let Some(listener) = &caller_focus_out {
                listener.call(event.into_any());
            }
            focus_exit_binding.focus_exit();
            if let Some(handler) = &on_focus_exit {
                handler.call(());
            }
        },
    });
    let merged = merge_attributes(vec![merged, interaction]);

    rsx! {
        primitive::Checkbox {
            checked,
            default_checked: default_value,
            required: resolved_required,
            disabled: resolved_disabled,
            name: resolved_name,
            value: form_value,
            on_checked_change: move |next| {
                change_binding.write(next, ChangeOrigin::User);
                if let Some(handler) = &on_change {
                    handler.call(next);
                }
                commit_binding.commit();
                if let Some(handler) = &on_commit {
                    handler.call(());
                }
                if let Some(control) = control() {
                    spawn(async move {
                        let _ = control.set_focus(true).await;
                    });
                }
            },
            attributes: merged,
        }
    }
}

fn take_event_listener(attributes: &mut Vec<Attribute>, name: &str) -> Option<ListenerCallback> {
    let index = attributes.iter().position(|attribute| {
        attribute.name == name && matches!(attribute.value, AttributeValue::Listener(_))
    })?;
    match attributes.remove(index).value {
        AttributeValue::Listener(listener) => Some(listener),
        _ => unreachable!(),
    }
}

/// The common Field composition for a checkbox.
///
/// This Composition sugar intentionally has no children. Use [`Field`] and its
/// Compound parts for inline or custom layouts, or when content or attributes
/// must land between the parts. Global attributes and caller classes are
/// forwarded to [`Checkbox`].
#[component]
pub fn CheckboxField(
    /// The context supplied to the checkbox and every Field part.
    #[props(into)]
    context: FieldContext,
    /// The checkbox's visible label.
    label: String,
    /// Supporting text rendered between the checkbox and its error region.
    #[props(default)]
    description: Option<String>,
    /// An explicit colour, or `None` to derive error colour from Field metadata.
    #[props(default)]
    color: Option<CheckboxColor>,
    /// daisyUI's size axis.
    #[props(default)]
    size: CheckboxSize,
    /// Whether the surrounding Field emits its default layout utilities.
    #[props(default)]
    field_appearance: FieldAppearance,
    /// Whether the control-and-label row emits its default layout utilities.
    #[props(default)]
    row_appearance: FieldRowAppearance,
    /// Whether supporting text emits its default wrapping utilities.
    #[props(default)]
    description_appearance: FieldDescriptionAppearance,
    /// Whether the error region emits its default semantic colour.
    #[props(default)]
    error_appearance: FieldErrorAppearance,
    /// An explicit Field binding, which wins over `context` for the checkbox.
    binding: Option<Binding<CheckboxState>>,
    /// An explicit two-state Field binding, used when `binding` is absent.
    bool_binding: Option<Binding<bool>>,
    /// Explicit Field metadata, which wins over `context` for the checkbox.
    meta: Option<FieldMeta>,
    /// The controlled value of the checkbox.
    #[props(default)]
    value: ReadSignal<Option<CheckboxState>>,
    /// The state the checkbox starts in when it is not controlled.
    #[props(default = CheckboxState::Unchecked)]
    default_value: CheckboxState,
    /// Whether the checkbox is required in a form.
    #[props(default)]
    required: Option<bool>,
    /// Whether the checkbox is disabled.
    #[props(default)]
    disabled: Option<bool>,
    /// The name of the checkbox, used in forms.
    #[props(default)]
    name: Option<String>,
    /// The submitted value of the checkbox.
    #[props(default = ReadSignal::new(Signal::new(String::from("on"))))]
    form_value: ReadSignal<String>,
    /// Called with the checkbox's value after user interaction.
    on_change: Option<EventHandler<CheckboxState>>,
    /// Called after every change ends its interaction unit.
    on_commit: Option<EventHandler<()>>,
    /// Called after focus leaves the checkbox button.
    on_focus_exit: Option<EventHandler<()>>,
    #[props(extends = GlobalAttributes)] attributes: Vec<Attribute>,
) -> Element {
    rsx! {
        Field { context, appearance: field_appearance,
            FieldRow { appearance: row_appearance,
                Checkbox {
                    color,
                    size,
                    binding,
                    bool_binding,
                    meta,
                    value,
                    default_value,
                    required,
                    disabled,
                    name,
                    form_value,
                    on_change,
                    on_commit,
                    on_focus_exit,
                    attributes,
                }
                FieldLabel { {label} }
            }
            if let Some(description) = description {
                FieldDescription { appearance: description_appearance, {description} }
            }
            FieldError { appearance: error_appearance }
        }
    }
}
