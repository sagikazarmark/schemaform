//! The daisyUI homogeneous-array chrome: a fieldset around item cards, with the item affordances
//! as a join of square icon buttons and the append and container presence affordances as buttons.

use dioxus::prelude::*;
use schemaform_dioxus::{
    CollectionContext, CollectionItemContext, CollectionRenderer,
    render::{Affordance, AffordanceKind},
};

use super::Appearance;
use super::parts::{icon, presence_affordances};
use crate::components::button::{Button, ButtonSize};

/// What an empty collection says in place of its items. A fixed host string: the localized item
/// noun the context carries is a name, not a word a sentence can be built around.
const EMPTY_STATE: &str = "Nothing here yet.";

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
///
/// A built-in fixed object rendered as an item keeps its own frame here; the theme that gives
/// `.schemaform-group` a frame is the one to flatten it inside a card, as the demo's does.
///
/// The [`Appearance`] axis switches every layout utility off; the `fieldset`, `card`, `btn`, and
/// `join` component classes always render.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaisyuiCollection {
    appearance: Appearance,
}

impl DaisyuiCollection {
    /// The same renderer at `appearance`.
    pub fn appearance(self, appearance: Appearance) -> Self {
        Self { appearance }
    }
}

impl CollectionRenderer for DaisyuiCollection {
    fn collection(&self, context: CollectionContext) -> Element {
        let appearance = self.appearance;
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
        let fieldset_class = appearance
            .utilities("min-w-0 gap-3 rounded-box border border-base-300 bg-base-100 p-4");
        let help_class = appearance.utilities("min-w-0 text-base-content/70");
        let readout_class = appearance.utilities("min-w-0 text-warning");
        let empty_class = appearance.utilities(
            "rounded-box border border-dashed border-base-300 p-4 text-center text-base-content/70",
        );
        let items_class = appearance.utilities("grid gap-3");
        let append_class = appearance.utilities("w-fit");
        let announcement_class = appearance.utilities("text-xs text-base-content/70");
        rsx! {
            fieldset {
                id: element_id.clone(),
                class: "fieldset {fieldset_class}",
                "data-schemaform-daisyui": "collection",
                tabindex: "-1",
                "aria-invalid": presentation.invalid,
                "aria-describedby": described_by,
                legend { id: "{element_id}-legend", class: legend_class, "{presentation.label}" }
                if let Some(help) = help {
                    p { id: help.id, class: help_class, "{help.text}" }
                }
                if let Some(value) = incompatible_value {
                    output { class: readout_class, "data-incompatible-value": "", "{value}" }
                }
                {presence_affordances(&presentation.presence, appearance)}
                if empty {
                    p {
                        class: empty_class,
                        "data-schemaform-daisyui": "collection-empty",
                        "{EMPTY_STATE}"
                    }
                }
                div { class: items_class, {context.items} }
                if let Some(append) = context.append {
                    {append_button(append, append_class)}
                }
                div { class: announcement_class, {context.announcement} }
                {findings}
            }
        }
    }

    fn collection_item(&self, context: CollectionItemContext) -> Element {
        let appearance = self.appearance;
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
        let card_class = appearance.utilities("bg-base-100");
        let body_class = appearance.utilities("gap-3");
        let header_class =
            appearance.utilities("flex flex-wrap items-center justify-between gap-2");
        let title_class = appearance
            .utilities("text-xs font-medium tracking-wide text-base-content/70 uppercase");
        rsx! {
            div {
                class: "card card-border card-sm {card_class}",
                role: "group",
                "aria-labelledby": title_id.clone(),
                "data-schemaform-daisyui": "collection-item",
                div { class: "card-body {body_class}",
                    div { class: header_class,
                        span {
                            id: title_id,
                            class: title_class,
                            "{context.item_label} {context.position}"
                        }
                        if !actions.is_empty() {
                            div { class: "join",
                                for affordance in actions {
                                    {item_affordance(affordance, appearance)}
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

/// The append affordance as an outline button below the items.
fn append_button(append: Affordance, class: &'static str) -> Element {
    let invoke = append.clone();
    rsx! {
        Button {
            id: append.id.clone(),
            r#type: "button",
            size: ButtonSize::Sm,
            class: "btn-outline {class}",
            onclick: move |_| invoke.invoke(),
            "{append.label}"
        }
    }
}

/// One item affordance as a square icon button in the item's `join`.
///
/// The button carries the affordance id (focus after a move targets it), the positional
/// accessible name as its `aria-label`, and the shorter visible label as its `title`, so the
/// icon-only button still reads in full to assistive technology and on hover.
fn item_affordance(affordance: Affordance, appearance: Appearance) -> Element {
    let name = affordance
        .accessible_name
        .clone()
        .unwrap_or_else(|| affordance.label.clone());
    let glyph = icon_path(affordance.kind);
    let invoke = affordance.clone();
    rsx! {
        Button {
            key: "{affordance.id}",
            id: affordance.id.clone(),
            r#type: "button",
            size: ButtonSize::Sm,
            class: "join-item btn-square",
            "aria-label": name,
            title: affordance.label.clone(),
            onclick: move |_| invoke.invoke(),
            if let Some(path) = glyph {
                {icon(path, appearance.utilities("size-4"))}
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
