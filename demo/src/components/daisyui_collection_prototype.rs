//! PROTOTYPE — throwaway spike for schemaform#16 (collection seam). Not production code.
//!
//! A daisyUI-styled `CollectionRenderer`: the array is a daisyUI `fieldset`, each item is a
//! `card` with a `join` of icon buttons for the four item affordances, the append affordance is a
//! primary `btn`, and the adapter's live region is placed visually hidden. Built against the draft
//! `CollectionContext` / `CollectionItemContext` types to find out what the contract must carry.
//!
//! Everything here is raw daisyUI class strings; the registry components (`card`, `join`, `button`)
//! are not installed in the demo yet and are only class wrappers anyway.

use dioxus::prelude::*;
use schemaform_dioxus::{
    Affordance, AffordanceKind, CollectionContext, CollectionItemContext, CollectionRenderer,
};

/// PROTOTYPE daisyUI collection chrome.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaisyCollection;

impl CollectionRenderer for DaisyCollection {
    fn collection(&self, context: CollectionContext) -> Element {
        let presentation = &context.presentation;
        let described_by = presentation.described_by();
        let findings = presentation.present_findings();
        let help = presentation.help.clone();
        let presence = presentation.presence.clone();
        let empty = context.count == 0;
        rsx! {
            fieldset {
                id: presentation.element_id.clone(),
                class: "fieldset bg-base-200 border-base-300 rounded-box border p-4 gap-3",
                "aria-invalid": presentation.invalid,
                "aria-describedby": described_by,
                tabindex: "-1",
                legend { class: "fieldset-legend", "{presentation.label}" }
                if let Some(help) = help {
                    p { id: help.id, class: "label", "{help.text}" }
                }
                if context.incompatible_value.is_some() || !presence.is_empty() {
                    div { class: "flex flex-wrap items-center gap-2",
                        if let Some(value) = context.incompatible_value {
                            kbd { class: "kbd kbd-sm", "{value}" }
                        }
                        div { class: "join",
                            for affordance in presence {
                                DaisyAffordanceButton {
                                    affordance,
                                    class: "btn btn-sm btn-outline join-item",
                                    icon_only: false,
                                }
                            }
                        }
                    }
                }
                if empty {
                    div { role: "note", class: "alert alert-soft text-sm",
                        // Renderer-authored copy: the host owns its localization. Composing the
                        // item noun into a sentence here is grammatically unsafe across locales.
                        "Nothing here yet."
                    }
                }
                div { class: "grid gap-3", {context.items} }
                if let Some(append) = context.append {
                    div {
                        DaisyAffordanceButton {
                            affordance: append,
                            class: "btn btn-primary btn-sm",
                            icon_only: false,
                        }
                    }
                }
                // The adapter's live region: placed, never restyled beyond hiding it visually.
                div { class: "sr-only", {context.announcement} }
                div { class: "text-error text-sm", {findings} }
            }
        }
    }

    fn collection_item(&self, context: CollectionItemContext) -> Element {
        rsx! {
            DaisyCollectionItem { context }
        }
    }
}

/// The item card. A child component so the `PartialEq` context flows as props and hooks could
/// be added here (none are needed).
#[component]
fn DaisyCollectionItem(context: CollectionItemContext) -> Element {
    // `row_id` belongs to the adapter's wrapper; derive a renderer-owned id from it.
    let title_id = format!("{}-title", context.row_id);
    let actions = [
        context.move_up.clone(),
        context.move_down.clone(),
        context.insert_before.clone(),
        context.remove.clone(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    rsx! {
        div {
            class: "card card-sm card-border bg-base-100",
            role: "group",
            "aria-labelledby": title_id.clone(),
            div { class: "card-body gap-3",
                div { class: "flex items-center justify-between gap-2",
                    span { id: title_id, class: "badge badge-neutral badge-sm",
                        "{context.item_label} {context.position} / {context.count}"
                    }
                    if !actions.is_empty() {
                        div { class: "join",
                            for affordance in actions {
                                DaisyAffordanceButton {
                                    affordance,
                                    class: "btn btn-sm btn-square btn-ghost join-item",
                                    icon_only: true,
                                }
                            }
                        }
                    }
                }
                {context.children.clone()}
            }
        }
    }
}

/// One affordance as a daisyUI button.
///
/// The button carries the affordance id (focus-after-mutation targets it), `aria-label` from the
/// positional accessible name when present, and either the label text or an icon with the label as
/// a tooltip.
#[component]
fn DaisyAffordanceButton(affordance: Affordance, class: &'static str, icon_only: bool) -> Element {
    let glyph = match affordance.kind {
        AffordanceKind::MoveUp => "\u{2191}",
        AffordanceKind::MoveDown => "\u{2193}",
        AffordanceKind::InsertBefore => "\u{2295}",
        AffordanceKind::RemoveItem => "\u{2715}",
        AffordanceKind::Append => "+",
        _ => "\u{22EF}",
    };
    let label = affordance.label.clone();
    let accessible_name = affordance
        .accessible_name
        .clone()
        .or_else(|| icon_only.then(|| label.clone()));
    let invoke = affordance.invoke;
    rsx! {
        button {
            id: affordance.id.clone(),
            r#type: "button",
            class: if icon_only { "{class} tooltip" } else { "{class}" },
            "data-tip": icon_only.then(|| label.clone()),
            "aria-label": accessible_name,
            onclick: move |_| invoke.call(()),
            if icon_only { "{glyph}" } else { "{label}" }
        }
    }
}
