use std::sync::Arc;

use dioxus::prelude::*;
use schemaform::WidgetSymbol;
use schemaform_dioxus::{
    BuiltinControlRenderer, ControlRegistry, RenderConfiguration, SchemaForm, use_form,
};

use crate::components::StatusLine;
use crate::components::schemaform_daisyui::{RADIO_WIDGET, SELECT_WIDGET};
use crate::examples::daisyui::{baseline_form_data, definition, redacted_submission_text};

/// The daisyUI example's definition and baseline through the built-in renderer alone: every node
/// is the adapter's, and nothing themes it.
///
/// The UI schema names the two daisyUI widget symbols, which the built-in registry does not know,
/// so the registry maps each of them onto the built-in control renderer; a choice asked to be a
/// radio group or a compound select is then the built-in native select.
///
/// The `data-schemaform-unstyled` wrapper opts the form out of the demo's daisyUI stylesheet,
/// which otherwise themes the built-in class hooks site-wide, and puts back the browser's own
/// form-control styling that Tailwind's preflight resets. What is left is the adapter's markup
/// with no theme: semantic HTML, `schemaform-*` class hooks, and the same structural behaviour as
/// the daisyUI page, for a side-by-side comparison.
#[component]
pub fn BuiltinComparisonExample() -> Element {
    let definition = use_hook(definition);
    let form = use_form(definition, baseline_form_data())
        .expect("the built-in comparison form should be created");
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .controls(built_in_controls())
            .build()
            .bind(&bound_form)
            .expect("the built-in renderer should bind every control")
    });
    let mut submitted = use_signal(String::new);
    let reset_form = form.clone();

    rsx! {
        div { class: "space-y-4", "data-schemaform-unstyled": "",
            SchemaForm {
                form: bound,
                on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                    submitted.set(redacted_submission_text(snapshot.form_data()));
                },
                on_error: move |error| crate::examples::report_form_error(&error),
            }
            button {
                class: "btn btn-sm btn-ghost",
                r#type: "button",
                onclick: move |_| {
                    if reset_form.reset().is_ok() {
                        submitted.set(String::new());
                    }
                },
                "Reset to baseline"
            }
            StatusLine { status: submitted.read().clone() }
        }
    }
}

/// The built-in registry, extended so the daisyUI widget symbols resolve to the built-in control
/// renderer instead of failing to bind.
fn built_in_controls() -> ControlRegistry {
    [RADIO_WIDGET, SELECT_WIDGET].into_iter().fold(
        ControlRegistry::with_builtins(),
        |registry, symbol| {
            registry.widget(
                WidgetSymbol::parse(symbol).expect("the daisyUI widget symbols are non-empty"),
                Arc::new(BuiltinControlRenderer),
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use dioxus::core::{NoOpMutations, VirtualDom};

    /// Mounts the example as the browser would and returns the markup it settles on.
    fn render() -> String {
        let mut dom = VirtualDom::new(super::BuiltinComparisonExample);
        dom.rebuild_in_place();
        for _ in 0..4 {
            dom.render_immediate(&mut NoOpMutations);
        }
        let html = dioxus_ssr::render(&dom);
        assert!(!html.contains("Encountered panic"), "{html}");
        html
    }

    /// The comparison renders the daisyUI example's definition and baseline, but every control
    /// is the built-in one and the daisyUI renderer never runs.
    #[test]
    fn the_comparison_renders_the_same_form_through_the_built_in_renderer() {
        let html = render();

        assert!(html.contains("class=\"schemaform-control\""), "{html}");
        assert!(!html.contains("data-schemaform-daisyui"), "{html}");
        assert!(!html.contains("role=\"radiogroup\""), "{html}");
        assert!(html.contains("value=\"Ada\""), "{html}");
        assert!(html.contains("<legend>Billing address</legend>"), "{html}");
        assert!(html.contains("class=\"schemaform-tabs\""), "{html}");
    }

    /// The wrapper carries the marker the demo stylesheet keys its opt-out on.
    #[test]
    fn the_comparison_opts_out_of_the_daisyui_theme() {
        assert!(render().contains("data-schemaform-unstyled"));
    }
}
