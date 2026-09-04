use dioxus::prelude::*;
use dioxus_code::{Code, code};

use crate::app::Route;
use crate::components::{
    DocsCallout, ExampleSection, ExternalAction, InlineCode, PageHeader, SourcePanel, snippet_theme,
};
use crate::examples::daisyui::{DaisyuiFormExample, WritingDirection};

/// The daisyUI theme for the class hooks of the built-in structure no seam exists for yet, quoted
/// on the page exactly as the site stylesheet compiles it.
const HOOK_THEME: &str = include_str!("../forms.css");

#[component]
pub fn Daisyui() -> Element {
    rsx! {
        DaisyuiPage { direction: WritingDirection::Ltr }
    }
}

#[component]
pub fn DaisyuiRtl() -> Element {
    rsx! {
        DaisyuiPage { direction: WritingDirection::Rtl }
    }
}

/// The variants of the daisyUI form the gallery offers, as a strip of links with the current
/// one marked. Shared with the built-in comparison page.
#[component]
pub fn DaisyuiVariants(current: Route) -> Element {
    let variants = [
        (Route::Daisyui {}, "Left to right"),
        (Route::DaisyuiRtl {}, "Right to left"),
        (Route::DaisyuiBuiltin {}, "Unstyled built-in"),
    ];
    rsx! {
        nav { "aria-label": "Form variants", class: "mt-6",
            div { class: "join",
                for (route , label) in variants {
                    Link {
                        to: route.clone(),
                        class: if route == current { "btn btn-sm join-item btn-active" } else { "btn btn-sm join-item" },
                        "aria-current": (route == current).then_some("page"),
                        "{label}"
                    }
                }
            }
        }
    }
}

#[component]
fn DaisyuiPage(direction: WritingDirection) -> Element {
    let (title, current) = match direction {
        WritingDirection::Ltr => ("daisyUI form", Route::Daisyui {}),
        WritingDirection::Rtl => ("daisyUI form, right to left", Route::DaisyuiRtl {}),
    };
    rsx! {
        PageHeader {
            eyebrow: "Renderers",
            title,
            intro: "A custom control renderer owns its whole control region, a collection renderer the array chrome, a shell renderer the summary placement and the submit button, and a finding presenter the summary alert; the built-in structure no seam exists for yet is styled with daisyUI classes through its class hooks. The whole form follows the site theme; the right-to-left variant is the same form under dir=\"rtl\".",
        }
        DaisyuiVariants { current }
        ExampleSection {
            title: "Registry widgets, daisyUI arrays and shell",
            intro: rsx! {
                "The "
                InlineCode { "schemaform_daisyui" }
                " component registers one "
                InlineCode { "ControlRenderer" }
                " above the built-in priority for every control kind, plus one per widget symbol, and maps the headless edit hooks onto "
                InlineCode { "dioxus-field" }
                " bindings. Strings, numbers, and integers are an "
                InlineCode { "Input" }
                "; a non-nullable boolean is a native checkbox and a nullable one the registry "
                InlineCode { "Checkbox" }
                " showing null as indeterminate; a write-only boolean or choice is a replacement select; choices are a "
                InlineCode { "NativeSelect" }
                " unless the UI schema names "
                InlineCode { "daisyui:radio" }
                " or "
                InlineCode { "daisyui:select" }
                "; constants are read-only output. Its "
                InlineCode { "structure()" }
                " bundle renders the team and tag arrays as cards with joined item actions and the form shell with a primary submit button, and its "
                InlineCode { "findings()" }
                " presenter frames the summary as an alert. The tabs, the identity group and its grid, and the billing address with its remove operation are the built-in renderer's, dressed by the stylesheet below. Try a two-character name, switch to another tab, and submit: the summary lists the finding, and its button reveals the tab and focuses the control."
            },
            demo: rsx! { DaisyuiFormExample { direction } },
            code: rsx! {
                Code { src: code!("src/examples/daisyui.rs"), theme: snippet_theme() }
            },
        }
        section { class: "mt-10 rounded-[2rem] border border-base-300 bg-base-100 p-6 shadow-sm sm:p-8",
            h2 { class: "text-xl font-semibold tracking-tight", "daisyUI classes on the remaining class hooks" }
            p { class: "mt-2 max-w-[70ch] text-sm leading-6 text-base-content/65",
                "The structure no renderer seam exists for yet — the built-in groups, tabs, layouts, and their presence operations, plus the built-in controls on the other gallery pages — takes its look from this Tailwind partial. It applies daisyUI's "
                InlineCode { "fieldset" }
                ", "
                InlineCode { "input" }
                ", "
                InlineCode { "checkbox" }
                ", and "
                InlineCode { "btn" }
                " classes to the "
                InlineCode { "schemaform-*" }
                " hooks and data markers the adapter emits, so every colour follows the active theme and every inline measurement is logical. Arrays, the finding summary, and the submit button no longer appear here: the collection and shell renderers and the finding presenter render them as daisyUI components directly. Hand-written rules are down to what no class expresses: the tab buttons, because daisyUI scopes "
                InlineCode { "tab" }
                " to a literal "
                InlineCode { ".tabs > .tab" }
                " pair that "
                InlineCode { "@apply" }
                " cannot re-target, and the row that seats a checkbox beside its label."
            }
            div { class: "mt-6 max-h-[32rem] overflow-auto", SourcePanel { source: HOOK_THEME } }
        }
        DocsCallout {
            title: "The component and the theme live in the demo",
            action: Some(ExternalAction::new(
                "Read the component's README",
                "https://github.com/sagikazarmark/schemaform/blob/main/demo/src/components/schemaform_daisyui/README.md",
            )),
            "The published crates do not depend on dioxus-field, the registry, or daisyUI. The component — its control renderer, structure bundle, and finding presenter — is laid out as a dx components member under src/components so it can move to a registry later, the theme for the remaining hooks is the demo's own stylesheet, and both are browser-CSR only."
        }
    }
}
