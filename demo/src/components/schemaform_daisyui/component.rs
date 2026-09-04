//! The daisyUI control renderer and the registry that selects it.

use std::sync::Arc;

use dioxus::prelude::*;
use schemaform::{
    WidgetSymbol,
    definition::{DefinitionNodeView, SemanticKind},
};
use schemaform_dioxus::{
    BuiltinControlRenderer, ControlKind, ControlMatcher, ControlRegistry, ControlRenderContext,
    ControlRenderer, StructureRenderers, render::BUILTIN_CONTROL_PRIORITY,
};

use super::boolean::BooleanControl;
use super::choice::{NativeSelectControl, RadioGroupControl, SelectControl};
use super::collection::DaisyuiCollection;
use super::constant::ConstantControl;
use super::shell::DaisyuiShell;
use super::text::TextControl;

/// Matcher priority at which [`controls`] registers [`DaisyuiControlRenderer`].
///
/// Above [`BUILTIN_CONTROL_PRIORITY`] so the daisyUI renderer wins every control it accepts
/// and the built-in keeps the rest.
pub const DAISYUI_CONTROL_PRIORITY: i32 = BUILTIN_CONTROL_PRIORITY + 10;

/// The widget symbol a UI schema control names to render a choice as a radio group.
pub const RADIO_WIDGET: &str = "daisyui:radio";

/// The widget symbol a UI schema control names to render a choice as the registry's compound
/// select rather than the native one.
pub const SELECT_WIDGET: &str = "daisyui:select";

/// A control registry in which every control kind renders as a daisyUI field.
///
/// The registry starts from the built-ins, so structural nodes keep their built-in
/// presentation and a control the daisyUI renderer does not accept still renders. Choices render
/// as a native select unless the UI schema names [`RADIO_WIDGET`] or [`SELECT_WIDGET`] for them.
pub fn controls() -> ControlRegistry {
    ControlRegistry::with_builtins()
        .matcher(
            DAISYUI_CONTROL_PRIORITY,
            Arc::new(DaisyuiControls),
            Arc::new(DaisyuiControlRenderer::default()),
        )
        .widget(
            widget_symbol(RADIO_WIDGET),
            Arc::new(DaisyuiControlRenderer::with_choice_widget(
                ChoiceWidget::RadioGroup,
            )),
        )
        .widget(
            widget_symbol(SELECT_WIDGET),
            Arc::new(DaisyuiControlRenderer::with_choice_widget(
                ChoiceWidget::Select,
            )),
        )
}

/// The registry key for one of this component's widget symbols.
fn widget_symbol(symbol: &str) -> WidgetSymbol {
    WidgetSymbol::parse(symbol).expect("the daisyUI widget symbols are non-empty")
}

/// The structure renderers this component ships: the daisyUI form shell and homogeneous-array
/// collection.
///
/// Every slot the component does not implement stays the built-in, so a form bound with this
/// bundle degrades to the adapter's accessible unstyled output for those node kinds rather than
/// losing a region.
pub fn structure() -> StructureRenderers {
    StructureRenderers::default()
        .with_shell(DaisyuiShell)
        .with_collection(DaisyuiCollection)
}

/// Accepts exactly the definition nodes [`DaisyuiControlRenderer`] presents itself: those the
/// adapter derives a control kind from.
struct DaisyuiControls;

impl ControlMatcher for DaisyuiControls {
    fn matches(&self, definition: DefinitionNodeView<'_>) -> bool {
        matches!(
            definition.semantic_kind(),
            Some(
                SemanticKind::String
                    | SemanticKind::Number
                    | SemanticKind::Integer
                    | SemanticKind::Boolean
                    | SemanticKind::Choice
                    | SemanticKind::Null
            )
        )
    }
}

/// The widget a [`DaisyuiControlRenderer`] presents a selectable choice with.
///
/// An exact widget request never reaches the renderer at render time, so the registry carries
/// one renderer per symbol and the symbol's meaning travels here.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChoiceWidget {
    /// The registry's `NativeSelect`: a native `select` at native-control weight.
    #[default]
    NativeSelect,
    /// The registry's `RadioGroup`: one `RadioItem` per option.
    RadioGroup,
    /// The registry's compound `Select`: a trigger and a dropdown listbox.
    Select,
}

/// Renders every control kind with the registry's `Field` parts and widgets.
///
/// The renderer owns the whole control region: label, widget, help, findings, and presence
/// affordances. Should a host register it for a control kind this component does not know, that
/// control is handed to [`BuiltinControlRenderer`] rather than to an editable widget the mapping
/// does not cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaisyuiControlRenderer {
    choice: ChoiceWidget,
}

impl DaisyuiControlRenderer {
    /// A renderer presenting choices with `choice`; [`Default`] presents them natively.
    pub fn with_choice_widget(choice: ChoiceWidget) -> Self {
        Self { choice }
    }
}

impl ControlRenderer for DaisyuiControlRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        // The kind is definition-stable, so a node always renders the same child component and
        // the hooks inside it are called unconditionally.
        match context.control().kind {
            ControlKind::String | ControlKind::Number | ControlKind::Integer => {
                rsx! { TextControl { context } }
            }
            ControlKind::Boolean => rsx! { BooleanControl { context } },
            ControlKind::Choice => match self.choice {
                ChoiceWidget::NativeSelect => rsx! { NativeSelectControl { context } },
                ChoiceWidget::RadioGroup => rsx! { RadioGroupControl { context } },
                ChoiceWidget::Select => rsx! { SelectControl { context } },
            },
            ControlKind::Constant => rsx! { ConstantControl { context } },
            _ => BuiltinControlRenderer.render(context),
        }
    }
}

#[cfg(test)]
mod tests {
    use dioxus::prelude::*;
    use schemaform::{CompilationProfile, FormDefinition, json::parse_ui_schema_v1};
    use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
    use serde_json::json;

    use super::controls;
    use crate::components::schemaform_daisyui::test_support::{
        RenderedForm, TestAppProps, assert_aria_references_resolve, tags,
    };

    /// The authored presentation: every control in data-schema order, except that the billing
    /// cycle asks for the radio widget and the region for the compound select.
    const UI_SCHEMA: &str = r#"{
      "version": 1,
      "root": {
        "type": "stack",
        "value": {
          "children": [
            {
              "type": "auto",
              "value": {
                "binding": { "origin": "root", "pointer": "" },
                "properties": { "exclude": ["billing", "region"] }
              }
            },
            {
              "type": "control",
              "value": {
                "binding": { "origin": "root", "pointer": "/billing" },
                "widget": "daisyui:radio"
              }
            },
            {
              "type": "control",
              "value": {
                "binding": { "origin": "root", "pointer": "/region" },
                "widget": "daisyui:select"
              }
            }
          ]
        }
      }
    }"#;

    fn gallery_app(props: TestAppProps) -> Element {
        let definition = use_hook(|| {
            let ui_schema =
                parse_ui_schema_v1(UI_SCHEMA.as_bytes(), &CompilationProfile::default())
                    .expect("the gallery UI schema should parse");
            FormDefinition::compiler(json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": false,
                "required": ["name", "quantity", "price", "active", "plan"],
                "properties": {
                    "name": {
                        "type": "string",
                        "title": "Name",
                        "description": "Shown on the badge.",
                        "minLength": 2
                    },
                    "quantity": { "type": "integer", "title": "Quantity" },
                    "price": { "type": "number", "title": "Price" },
                    "nickname": { "type": ["string", "null"], "title": "Nickname" },
                    "secret": { "type": "string", "title": "Secret", "writeOnly": true },
                    "reference": {
                        "type": "string",
                        "title": "Reference",
                        "description": "Assigned by the server.",
                        "readOnly": true
                    },
                    "active": { "type": "boolean", "title": "Active" },
                    "newsletter": { "type": ["boolean", "null"], "title": "Newsletter" },
                    "mfa": { "type": "boolean", "title": "MFA", "writeOnly": true },
                    "plan": {
                        "type": ["string", "null"],
                        "title": "Plan",
                        "enum": ["starter", "team", null]
                    },
                    "recovery": {
                        "title": "Recovery",
                        "enum": ["email", "sms"],
                        "writeOnly": true
                    },
                    "tier": { "title": "Tier", "const": "standard" },
                    "billing": {
                        "type": ["string", "null"],
                        "title": "Billing",
                        "enum": ["monthly", "yearly", null]
                    },
                    "region": {
                        "type": ["string", "null"],
                        "title": "Region",
                        "enum": ["eu", "us", null]
                    }
                }
            }))
            .ui_schema(ui_schema)
            .compile()
            .expect("the gallery data schema should compile")
        });
        let form = use_form(
            definition,
            json!({
                "name": "Ada",
                "quantity": 1,
                "price": 9.5,
                "nickname": null,
                "secret": "hunter2",
                "reference": "ref_42",
                "active": true,
                "newsletter": null,
                "mfa": true,
                "plan": "team",
                "recovery": "sms",
                "tier": "standard",
                "billing": "yearly",
                "region": "eu"
            }),
        )
        .expect("the gallery form should be created");
        props
            .handle
            .borrow_mut()
            .get_or_insert_with(|| form.clone());
        let bound = use_hook(move || {
            RenderConfiguration::builder()
                .controls(controls())
                .build()
                .bind(&form)
                .expect("the daisyUI registry should bind every control")
        });
        rsx! {
            SchemaForm { form: bound, on_submit: move |_| {} }
        }
    }

    /// The gallery form, mounted and settled.
    fn mount() -> RenderedForm {
        RenderedForm::mount(gallery_app)
    }

    #[test]
    fn string_number_and_integer_controls_render_as_daisyui_inputs() {
        let rendered = mount();

        for (name, inputmode) in [
            ("/name", "text"),
            ("/quantity", "numeric"),
            ("/price", "decimal"),
        ] {
            let control = rendered.control(name);
            assert_eq!(control.element, "input", "{name} should be an input");
            assert!(
                control.classes().contains(&"input"),
                "{name} should carry daisyUI's input class: {control:?}"
            );
            assert_eq!(control.attribute("type"), Some("text"), "{name}");
            assert_eq!(control.attribute("inputmode"), Some(inputmode), "{name}");
            assert_eq!(control.attribute("required"), Some("true"), "{name}");
        }
    }

    #[test]
    fn a_non_nullable_boolean_is_a_native_checkbox_with_the_daisyui_class() {
        let rendered = mount();

        let active = rendered.control("/active");
        assert_eq!(active.element, "input");
        assert_eq!(active.attribute("type"), Some("checkbox"));
        assert!(
            active.classes().contains(&"checkbox"),
            "the checkbox should carry daisyUI's class: {active:?}"
        );
        assert_eq!(active.attribute("checked"), Some("true"));
        assert_eq!(active.attribute("aria-required"), Some("true"));
        assert_eq!(active.attribute("aria-invalid"), Some("false"));
        assert_eq!(
            active.attribute("id").map(str::to_owned),
            Some(rendered.control_id("/active"))
        );
    }

    #[test]
    fn a_nullable_boolean_is_the_registry_checkbox_showing_null_as_indeterminate() {
        let mut rendered = mount();

        let newsletter = rendered.control("/newsletter");
        assert_eq!(newsletter.element, "button");
        assert_eq!(newsletter.attribute("role"), Some("checkbox"));
        assert!(
            newsletter.classes().contains(&"checkbox"),
            "the checkbox should carry daisyUI's class: {newsletter:?}"
        );
        assert_eq!(newsletter.attribute("aria-checked"), Some("mixed"));
        assert_eq!(
            newsletter.attribute("id").map(str::to_owned),
            Some(rendered.control_id("/newsletter"))
        );

        let actions = rendered.actions_at("/newsletter");
        actions
            .set_value(json!(true))
            .expect("the boolean should be set");
        rendered.settle();

        assert_eq!(
            rendered.control("/newsletter").attribute("aria-checked"),
            Some("true")
        );
    }

    #[test]
    fn a_write_only_boolean_is_a_replacement_select_that_never_shows_its_value() {
        let rendered = mount();

        let mfa = rendered.control("/mfa");
        assert_eq!(mfa.element, "select");
        assert!(
            mfa.classes().contains(&"select"),
            "the replacement select should carry daisyUI's class: {mfa:?}"
        );
        assert_eq!(mfa.attribute("data-write-only-replacement"), Some(""));
        assert_eq!(mfa.attribute("value"), Some(""));
        let html = rendered.html();
        assert!(html.contains("Replace MFA"), "{html}");
        let options = rendered.options("/mfa");
        assert_eq!(
            options,
            vec![
                ("".to_owned(), "Choose replacement".to_owned(), true),
                ("false".to_owned(), "False".to_owned(), false),
                ("true".to_owned(), "True".to_owned(), false),
            ]
        );
    }

    #[test]
    fn a_choice_is_a_daisyui_native_select_over_opaque_identities_with_the_null_option() {
        let mut rendered = mount();

        let plan = rendered.control("/plan");
        assert_eq!(plan.element, "select");
        assert!(
            plan.classes().contains(&"select"),
            "the select should carry daisyUI's class: {plan:?}"
        );
        assert_eq!(plan.attribute("required"), Some("true"));
        let options = rendered.options("/plan");
        let labels = options
            .iter()
            .map(|(_, label, _)| label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels, vec!["", "null", "starter", "team"]);
        let selected = options
            .iter()
            .filter(|(_, _, selected)| *selected)
            .map(|(_, label, _)| label.as_str())
            .collect::<Vec<_>>();
        assert_eq!(selected, vec!["team"]);
        let values = options
            .iter()
            .skip(1)
            .map(|(value, _, _)| value.as_str())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(
            values.len(),
            3,
            "option values should be distinct: {options:?}"
        );
        assert!(
            !values.contains(""),
            "only the placeholder is empty: {options:?}"
        );
        assert_eq!(plan.attribute("value"), options[3].0.as_str().into());

        let actions = rendered.actions_at("/plan");
        actions.set_null().expect("the choice should accept null");
        rendered.settle();

        let selected = rendered
            .options("/plan")
            .into_iter()
            .filter(|(_, _, selected)| *selected)
            .map(|(_, label, _)| label)
            .collect::<Vec<_>>();
        assert_eq!(selected, vec!["null"]);
    }

    #[test]
    fn a_write_only_choice_shows_the_replacement_placeholder_and_no_selection() {
        let rendered = mount();

        let recovery = rendered.control("/recovery");
        assert_eq!(recovery.element, "select");
        assert!(recovery.classes().contains(&"select"), "{recovery:?}");
        assert_eq!(recovery.attribute("data-write-only-replacement"), Some(""));
        assert_eq!(recovery.attribute("value"), Some(""));
        assert!(rendered.html().contains("Replace Recovery"));
        let options = rendered.options("/recovery");
        assert_eq!(options[0].1, "Choose replacement");
        assert!(
            options[0].2,
            "the placeholder should be selected: {options:?}"
        );
        assert!(
            options.iter().skip(1).all(|(_, _, selected)| !selected),
            "no option should be selected: {options:?}"
        );
    }

    #[test]
    fn the_radio_widget_symbol_selects_a_daisyui_radio_group_with_one_item_per_option() {
        let mut rendered = mount();

        let group = rendered
            .find(|tag| tag.attribute("role") == Some("radiogroup"))
            .expect("a radio group should be rendered");
        let group_id = group
            .attribute("id")
            .expect("the group should carry the control's element id")
            .to_owned();
        assert_eq!(group.attribute("aria-invalid"), Some("false"));
        assert!(
            group.attribute("aria-labelledby").is_some(),
            "the group should be labelled: {group:?}"
        );

        let items = rendered.find_all(|tag| tag.attribute("role") == Some("radio"));
        assert_eq!(
            items.len(),
            3,
            "one item per option, the null option included"
        );
        for item in &items {
            assert!(item.classes().contains(&"radio"), "{item:?}");
            assert!(item.attribute("aria-labelledby").is_some(), "{item:?}");
        }
        let checked = items
            .iter()
            .filter(|item| item.attribute("aria-checked") == Some("true"))
            .count();
        assert_eq!(checked, 1);
        let checked_id = items
            .iter()
            .find(|item| item.attribute("aria-checked") == Some("true"))
            .and_then(|item| item.attribute("id"))
            .expect("the checked item should carry an id")
            .to_owned();
        assert_eq!(rendered.labelled_by_text(&checked_id), "yearly");

        // The hidden form participants carry the control's name, one per item.
        let participants = rendered.find_all(|tag| {
            tag.element == "input"
                && tag.attribute("type") == Some("radio")
                && tag.attribute("name") == Some("/billing")
        });
        assert_eq!(participants.len(), 3);

        rendered
            .actions_at("/billing")
            .set_null()
            .expect("billing should accept null");
        rendered.settle();

        let checked_id = rendered
            .find_all(|tag| tag.attribute("role") == Some("radio"))
            .iter()
            .find(|item| item.attribute("aria-checked") == Some("true"))
            .and_then(|item| item.attribute("id"))
            .expect("the null item should be checked")
            .to_owned();
        assert_eq!(rendered.labelled_by_text(&checked_id), "null");
        assert_eq!(
            rendered
                .find(|tag| tag.attribute("role") == Some("radiogroup"))
                .and_then(|tag| tag.attribute("id").map(str::to_owned)),
            Some(group_id)
        );
    }

    #[test]
    fn a_constant_is_read_only_output_with_its_presence_affordances() {
        let rendered = mount();

        let tier = rendered.control("/tier");
        assert_eq!(tier.element, "output");
        assert_eq!(tier.attribute("data-schemaform-control"), Some("constant"));
        assert_eq!(tier.attribute("tabindex"), Some("-1"));
        assert_eq!(tier.attribute("aria-invalid"), Some("false"));
        let html = rendered.html();
        assert!(html.contains(">standard</output>"), "{html}");
        assert!(
            rendered
                .find(|tag| tag.attribute("name") == Some("/tier") && tag.element != "output")
                .is_none(),
            "a constant is never an editable widget: {html}"
        );

        let tier_id = rendered.control_id("/tier");
        let remove = rendered
            .find(|tag| tag.attribute("id") == Some(&format!("{tier_id}-remove-value")))
            .expect("an optional constant offers its remove affordance");
        assert_eq!(remove.element, "button");
        assert!(remove.classes().contains(&"btn"), "{remove:?}");
    }

    #[test]
    fn the_select_widget_symbol_selects_the_daisyui_compound_select() {
        let mut rendered = mount();

        let trigger = rendered.control("/region");
        assert_eq!(trigger.element, "button");
        assert_eq!(trigger.attribute("aria-haspopup"), Some("listbox"));
        assert!(
            trigger.classes().contains(&"select"),
            "the trigger should carry daisyUI's class: {trigger:?}"
        );
        assert_eq!(
            trigger.attribute("id").map(str::to_owned),
            Some(rendered.control_id("/region"))
        );
        assert_eq!(trigger.attribute("aria-expanded"), Some("false"));
        assert!(
            rendered.html().contains(">eu</span></button>"),
            "the trigger should show the selected option's label"
        );

        let actions = rendered.actions_at("/region");
        actions
            .set_value(json!("us"))
            .expect("region should accept us");
        rendered.settle();
        assert!(rendered.html().contains(">us</span></button>"));

        // The null option is an option like any other: selecting null shows its label.
        actions.set_null().expect("region should accept null");
        rendered.settle();
        assert!(rendered.html().contains(">null</span></button>"));
    }

    #[test]
    fn a_write_only_control_uses_the_password_type_and_the_replacement_label() {
        let rendered = mount();

        let secret = rendered.control("/secret");
        assert_eq!(secret.attribute("type"), Some("password"));
        assert_eq!(secret.attribute("value"), Some(""));
        assert_eq!(secret.attribute("placeholder"), Some("Choose replacement"));
        assert!(rendered.html().contains("Replace Secret"));
    }

    #[test]
    fn a_read_only_control_renders_as_output_rather_than_an_editable_input() {
        let rendered = mount();

        let reference = rendered.control("/reference");
        assert_eq!(reference.element, "output");
        assert_eq!(
            reference.attribute("aria-describedby"),
            Some(&*format!("{}-help", rendered.control_id("/reference")))
        );
        assert!(rendered.html().contains(">ref_42</output>"));
    }

    #[test]
    fn help_is_described_by_and_every_aria_reference_resolves_to_an_element() {
        let mut rendered = mount();
        // Surface an error so `aria-errormessage` is emitted too.
        let quantity = rendered.actions_at("/quantity");
        quantity
            .input_text("-")
            .expect("the parse blocker should be recorded");
        rendered.settle();

        let html = rendered.html();
        let name = rendered.control("/name");
        assert!(
            name.attribute("aria-describedby")
                .is_some_and(|value| value.contains("-help")),
            "help should describe the input: {name:?}"
        );
        assert_eq!(
            rendered.control("/quantity").attribute("aria-invalid"),
            Some("true")
        );
        assert!(
            rendered
                .control("/quantity")
                .attribute("aria-errormessage")
                .is_some()
        );

        assert!(assert_aria_references_resolve(&html) > 0);
    }

    #[test]
    fn presence_affordances_render_as_daisyui_buttons_carrying_their_ids() {
        let rendered = mount();
        let nickname = rendered.control_id("/nickname");

        let buttons = tags(&rendered.html())
            .into_iter()
            .filter(|tag| {
                tag.element == "button"
                    && tag
                        .attribute("id")
                        .is_some_and(|id| id.starts_with(&format!("{nickname}-")))
            })
            .collect::<Vec<_>>();

        let ids = buttons
            .iter()
            .filter_map(|button| button.attribute("id"))
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                format!("{nickname}-set-value"),
                format!("{nickname}-remove-value")
            ]
        );
        for button in &buttons {
            assert!(button.classes().contains(&"btn"), "{button:?}");
            assert_eq!(button.attribute("type"), Some("button"), "{button:?}");
        }
    }
}
