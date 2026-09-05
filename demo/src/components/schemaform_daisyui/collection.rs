//! The daisyUI homogeneous-array chrome: a fieldset around item cards, with the item affordances
//! as a join of square icon buttons and the append and container presence affordances as buttons.

use dioxus::prelude::*;
use schemaform_dioxus::{
    CollectionContext, CollectionItemContext, CollectionRenderer,
    render::{Affordance, AffordanceKind},
};

use super::parts::{icon, presence_affordances};
use crate::components::button::{Button, ButtonSize};

/// What an empty collection says in place of its items. A fixed host string: the localized item
/// noun the context carries is a name, not a word a sentence can be built around.
const EMPTY_STATE: &str = "Nothing here yet.";

/// The classes a card body applies to a built-in fixed-object group rendered as its item, so the
/// group's own box and legend flatten into the card instead of nesting a second frame under the
/// card's title. They name the adapter's class hook because there is no `FixedObjectRenderer`
/// seam yet; when one lands, the daisyUI fixed object decides its own frame and these go.
const FLATTEN_NESTED_GROUP: &str = "[&>.schemaform-group]:border-0 [&>.schemaform-group]:bg-transparent [&>.schemaform-group]:p-0 [&>.schemaform-group>legend]:sr-only";

/// Presents a homogeneous array as a daisyUI fieldset of item cards.
///
/// The fieldset carries the adapter's element id, is focusable for the container presence
/// operations, and describes itself by its help and findings. Inside it: the legend, the help,
/// the incompatible-value readout and container presence buttons, an empty state while there are
/// no items and no incompatible data to explain their absence, the adapter-keyed item hosts, the
/// append button, the adapter's live region, and the local findings. Each item is a card whose
/// header names the item and holds its insert, move, and remove affordances as a `join` of square
/// icon buttons named by their positional accessible names; the item's own controls follow.
///
/// Identity, keying, focus after a mutation, and announcements stay the adapter's: every button
/// carries its affordance id, so focus after a move lands on the same button in the moved row.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaisyuiCollection;

impl CollectionRenderer for DaisyuiCollection {
    fn collection(&self, context: CollectionContext) -> Element {
        let presentation = context.presentation;
        let element_id = presentation.element_id.clone();
        let described_by = presentation.described_by();
        let findings = presentation.present_findings();
        let help = presentation.help.clone();
        let incompatible_value = presentation.incompatible_value.clone();
        let empty = context.count == 0 && incompatible_value.is_none();
        let legend_class = if presentation.label_visible {
            "fieldset-legend"
        } else {
            "fieldset-legend sr-only"
        };
        rsx! {
            fieldset {
                id: element_id.clone(),
                class: "fieldset min-w-0 gap-3 rounded-box border border-base-300 bg-base-100 p-4",
                "data-schemaform-daisyui": "collection",
                tabindex: "-1",
                "aria-invalid": presentation.invalid,
                "aria-describedby": described_by,
                legend { id: "{element_id}-legend", class: legend_class, "{presentation.label}" }
                if let Some(help) = help {
                    p { id: help.id, class: "min-w-0 text-base-content/70", "{help.text}" }
                }
                if let Some(value) = incompatible_value {
                    output { class: "min-w-0 text-warning", "data-incompatible-value": "", "{value}" }
                }
                {presence_affordances(&presentation.presence)}
                if empty {
                    p {
                        class: "rounded-box border border-dashed border-base-300 p-4 text-center text-base-content/70",
                        "data-schemaform-daisyui": "collection-empty",
                        "{EMPTY_STATE}"
                    }
                }
                div { class: "grid gap-3", {context.items} }
                if let Some(append) = context.append {
                    Button {
                        id: append.id.clone(),
                        r#type: "button",
                        size: ButtonSize::Sm,
                        class: "btn-outline w-fit",
                        onclick: move |_| append.invoke.call(()),
                        "{append.label}"
                    }
                }
                div { class: "text-xs text-base-content/70", {context.announcement} }
                {findings}
            }
        }
    }

    fn collection_item(&self, context: CollectionItemContext) -> Element {
        let title_id = format!("{}-title", context.row_id);
        let actions = [
            context.insert_before,
            context.move_up,
            context.move_down,
            context.remove,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        rsx! {
            div {
                class: "card card-border card-sm bg-base-100",
                role: "group",
                "aria-labelledby": title_id.clone(),
                "data-schemaform-daisyui": "collection-item",
                div { class: "card-body gap-3 {FLATTEN_NESTED_GROUP}",
                    div { class: "flex flex-wrap items-center justify-between gap-2",
                        span {
                            id: title_id,
                            class: "text-xs font-medium tracking-wide text-base-content/70 uppercase",
                            "{context.item_label} {context.position}"
                        }
                        if !actions.is_empty() {
                            div { class: "join",
                                for affordance in actions {
                                    {item_affordance(affordance)}
                                }
                            }
                        }
                    }
                    {context.children}
                }
            }
        }
    }
}

/// One item affordance as a square icon button in the item's `join`.
///
/// The button carries the affordance id (focus after a move targets it), the positional
/// accessible name as its `aria-label`, and the shorter visible label as its `title`, so the
/// icon-only button still reads in full to assistive technology and on hover.
fn item_affordance(affordance: Affordance) -> Element {
    let name = affordance
        .accessible_name
        .clone()
        .unwrap_or_else(|| affordance.label.clone());
    let glyph = icon_path(affordance.kind);
    rsx! {
        Button {
            key: "{affordance.id}",
            id: affordance.id.clone(),
            r#type: "button",
            size: ButtonSize::Sm,
            class: "join-item btn-square",
            "aria-label": name,
            title: affordance.label.clone(),
            onclick: move |_| affordance.invoke.call(()),
            if let Some(path) = glyph {
                {icon(path, "size-4")}
            } else {
                "{affordance.label}"
            }
        }
    }
}

/// The Heroicons outline path for an item affordance; `None` for a kind the adapter does not
/// hand to `collection_item` today, which then shows its label instead.
fn icon_path(kind: AffordanceKind) -> Option<&'static str> {
    match kind {
        AffordanceKind::InsertBefore => Some("M12 4.5v15m7.5-7.5h-15"),
        AffordanceKind::MoveUp => Some("M4.5 10.5 12 3m0 0 7.5 7.5M12 3v18"),
        AffordanceKind::MoveDown => Some("M19.5 13.5 12 21m0 0-7.5-7.5M12 21V3"),
        AffordanceKind::RemoveItem => Some("M6 18 18 6M6 6l12 12"),
        _ => None,
    }
}
