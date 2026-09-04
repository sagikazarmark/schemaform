//! Temporary local Checkbox Primitive.
//!
//! Adapted from `DioxusLabs/components`' `primitives/src/checkbox.rs` at
//! revision `bf007c15d0cf4d04d3181cc46cf12325aa773955`, under MIT OR
//! Apache-2.0. This copy makes the form participant's name optional. Remove it
//! once the pinned Primitive can omit the attribute itself.

use std::{
    rc::Rc,
    sync::atomic::{AtomicUsize, Ordering},
};

use dioxus::{document::eval, prelude::*};
use dioxus_primitives::{checkbox::CheckboxState, use_controlled};

#[derive(Props, Clone, PartialEq)]
pub(super) struct CheckboxProps {
    pub checked: ReadSignal<Option<CheckboxState>>,
    #[props(default = CheckboxState::Unchecked)]
    pub default_checked: CheckboxState,
    #[props(default)]
    pub required: ReadSignal<bool>,
    #[props(default)]
    pub disabled: ReadSignal<bool>,
    #[props(default)]
    pub name: ReadSignal<Option<String>>,
    #[props(default = ReadSignal::new(Signal::new(String::from("on"))))]
    pub value: ReadSignal<String>,
    #[props(default)]
    pub on_checked_change: Callback<CheckboxState>,
    #[props(extends = GlobalAttributes)]
    pub attributes: Vec<Attribute>,
    pub children: Element,
}

#[component]
pub(super) fn Checkbox(props: CheckboxProps) -> Element {
    let (checked, set_checked) = use_controlled(
        props.checked,
        props.default_checked,
        props.on_checked_change,
    );
    let mut button_ref: Signal<Option<Rc<MountedData>>> = use_signal(|| None);

    rsx! {
        button {
            type: "button",
            value: props.value,
            role: "checkbox",
            aria_checked: aria_checked(checked()),
            aria_required: props.required,
            disabled: props.disabled,
            "data-state": data_state(checked()),
            "data-disabled": props.disabled,

            onmounted: move |event| button_ref.set(Some(event.data())),
            onclick: move |_| {
                let next = toggle(checked());
                set_checked.call(next);
                if let Some(node) = button_ref() {
                    spawn(async move {
                        let _ = node.set_focus(true).await;
                    });
                }
            },
            onkeydown: move |event| {
                if event.key() == Key::Enter {
                    event.prevent_default();
                }
            },

            ..props.attributes,
            {props.children}
        }
        BubbleInput {
            checked,
            default_checked: props.default_checked,
            required: props.required,
            name: props.name,
            value: props.value,
            disabled: props.disabled,
        }
    }
}

#[component]
fn BubbleInput(
    checked: ReadSignal<CheckboxState>,
    default_checked: CheckboxState,
    #[props(extends = input)] attributes: Vec<Attribute>,
) -> Element {
    let id = use_checkbox_id();

    use_effect(move || {
        let checked = checked();
        let js = eval(
            r#"
            let id = await dioxus.recv();
            let action = await dioxus.recv();
            let input = document.getElementById(id);

            switch(action) {
                case "checked":
                    input.checked = true;
                    input.indeterminate = false;
                    break;
                case "indeterminate":
                    input.indeterminate = true;
                    input.checked = true;
                    break;
                case "unchecked":
                    input.checked = false;
                    input.indeterminate = false;
                    break;
            }
            "#,
        );

        let _ = js.send(id());
        let _ = js.send(data_state(checked));
    });

    rsx! {
        input {
            id,
            type: "checkbox",
            aria_hidden: "true",
            tabindex: "-1",
            position: "absolute",
            pointer_events: "none",
            opacity: "0",
            margin: "0",
            transform: "translateX(-100%)",
            checked: default_checked != CheckboxState::Unchecked,
            ..attributes,
        }
    }
}

fn use_checkbox_id() -> Signal<String> {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

    #[allow(unused_mut)]
    let mut initial_value = use_hook(|| {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        format!("daisyui-checkbox-{id}")
    });
    fullstack! {
        let server_id = use_server_cached(move || initial_value.clone());
        initial_value = server_id;
    }
    use_signal(|| initial_value)
}

fn aria_checked(state: CheckboxState) -> &'static str {
    match state {
        CheckboxState::Checked => "true",
        CheckboxState::Indeterminate => "mixed",
        CheckboxState::Unchecked => "false",
    }
}

fn data_state(state: CheckboxState) -> &'static str {
    match state {
        CheckboxState::Checked => "checked",
        CheckboxState::Indeterminate => "indeterminate",
        CheckboxState::Unchecked => "unchecked",
    }
}

fn toggle(state: CheckboxState) -> CheckboxState {
    match state {
        CheckboxState::Unchecked => CheckboxState::Checked,
        CheckboxState::Checked | CheckboxState::Indeterminate => CheckboxState::Unchecked,
    }
}
