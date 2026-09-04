//! The daisyUI control renderer and the registry that selects it.

use std::sync::Arc;

use dioxus::prelude::*;
use schemaform::{
    WidgetSymbol,
    definition::{DefinitionNodeView, SemanticKind},
};
use schemaform_dioxus::{
    BuiltinControlRenderer, ControlKind, ControlMatcher, ControlRegistry, ControlRenderContext,
    ControlRenderer, render::BUILTIN_CONTROL_PRIORITY,
};

use super::boolean::BooleanControl;
use super::choice::{NativeSelectControl, RadioGroupControl, SelectControl};
use super::constant::ConstantControl;
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
    use std::{cell::RefCell, rc::Rc};

    use dioxus::core::{NoOpMutations, VirtualDom};
    use dioxus::prelude::*;
    use schemaform::{
        CompilationProfile, FormDefinition, InstanceIdentity, json::parse_ui_schema_v1,
    };
    use schemaform_dioxus::{FormHandle, RenderConfiguration, SchemaForm, use_form};
    use serde_json::json;

    use super::controls;

    #[derive(Clone, Props)]
    struct GalleryAppProps {
        handle: Rc<RefCell<Option<FormHandle>>>,
    }

    impl PartialEq for GalleryAppProps {
        fn eq(&self, other: &Self) -> bool {
            Rc::ptr_eq(&self.handle, &other.handle)
        }
    }

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

    fn gallery_app(props: GalleryAppProps) -> Element {
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

    /// The rendered form: the form handle plus the markup, observed as a browser would see it.
    struct RenderedGallery {
        dom: VirtualDom,
        handle: FormHandle,
    }

    impl RenderedGallery {
        fn mount() -> Self {
            let handle = Rc::new(RefCell::new(None));
            let mut dom = VirtualDom::new_with_props(
                gallery_app,
                GalleryAppProps {
                    handle: handle.clone(),
                },
            );
            dom.rebuild_in_place();
            let handle = handle
                .borrow()
                .clone()
                .expect("the gallery app should expose its form handle");
            let mut rendered = Self { dom, handle };
            rendered.settle();
            rendered
        }

        /// Field parts register their ids while they render and `Field` syncs metadata in an
        /// effect, so the control's ARIA references land on the renders that follow.
        fn settle(&mut self) {
            for _ in 0..4 {
                self.dom.render_immediate(&mut NoOpMutations);
            }
        }

        fn html(&self) -> String {
            dioxus_ssr::render(&self.dom)
        }

        /// The first tag `accept` accepts, in document order.
        fn find(&self, accept: impl Fn(&Tag) -> bool) -> Option<Tag> {
            tags(&self.html()).into_iter().find(accept)
        }

        /// Every tag `accept` accepts, in document order.
        fn find_all(&self, accept: impl Fn(&Tag) -> bool) -> Vec<Tag> {
            tags(&self.html()).into_iter().filter(accept).collect()
        }

        /// The text of the element that `aria-labelledby` of the element with `id` references.
        fn labelled_by_text(&self, id: &str) -> String {
            let label_id = self
                .find(|tag| tag.attribute("id") == Some(id))
                .and_then(|tag| tag.attribute("aria-labelledby").map(str::to_owned))
                .unwrap_or_else(|| panic!("{id} should be labelled"));
            let html = self.html();
            let start = html
                .find(&format!("id=\"{label_id}\""))
                .unwrap_or_else(|| panic!("the label {label_id} should exist"));
            let rest = &html[start..];
            let text_start = rest.find('>').expect("the label tag should close") + 1;
            let text_end = rest.find('<').expect("the label should close");
            rest[text_start..text_end].to_owned()
        }

        /// The `(value, text, selected)` of every option of the select named `name`, in order.
        fn options(&self, name: &str) -> Vec<(String, String, bool)> {
            let html = self.html();
            let start = html
                .find(&format!("name=\"{name}\""))
                .unwrap_or_else(|| panic!("a select named {name} should be rendered:\n{html}"));
            let select = &html[start..];
            let end = select
                .find("</select>")
                .unwrap_or_else(|| panic!("the select named {name} should close:\n{html}"));
            let select = &select[..end];
            let mut options = Vec::new();
            let mut rest = select;
            while let Some(start) = rest.find("<option") {
                rest = &rest[start..];
                let tag_end = rest.find('>').expect("an option tag should close");
                let tag = tags(&rest[..=tag_end])
                    .pop()
                    .expect("an option tag should parse");
                rest = &rest[tag_end + 1..];
                let text_end = rest.find("</option>").expect("an option should close");
                options.push((
                    tag.attribute("value").unwrap_or_default().to_owned(),
                    rest[..text_end].trim().to_owned(),
                    tag.attribute("selected") == Some("true"),
                ));
                rest = &rest[text_end..];
            }
            options
        }

        /// The attributes of the first tag whose `name` attribute is `name`.
        fn control(&self, name: &str) -> Tag {
            let html = self.html();
            tags(&html)
                .into_iter()
                .find(|tag| tag.attribute("name") == Some(name))
                .unwrap_or_else(|| panic!("a control named {name} should be rendered:\n{html}"))
        }

        /// The DOM id the adapter assigned to the control bound at `name`.
        fn control_id(&self, name: &str) -> String {
            self.control(name)
                .attribute("id")
                .unwrap_or_else(|| panic!("the control named {name} should carry an id"))
                .to_owned()
        }

        /// The instance identity of the control bound at `name`, for driving it through the
        /// form handle the way a host would.
        fn control_identity(&self, name: &str) -> InstanceIdentity {
            let root = self
                .handle
                .reader()
                .read()
                .expect("the form should be readable")
                .root;
            let mut pending = vec![root];
            while let Some(identity) = pending.pop() {
                let projection = self
                    .handle
                    .node(identity)
                    .expect("the form should be readable")
                    .expect("the node should exist")
                    .read()
                    .expect("the node should be readable")
                    .expect("the node should remain present");
                if projection
                    .binding
                    .as_ref()
                    .is_some_and(|pointer| pointer.as_str() == name)
                {
                    return identity;
                }
                pending.extend(projection.children);
            }
            panic!("a control bound at {name} should exist");
        }
    }

    /// One start tag from the rendered markup.
    #[derive(Debug)]
    struct Tag {
        element: String,
        attributes: Vec<(String, String)>,
    }

    impl Tag {
        fn attribute(&self, name: &str) -> Option<&str> {
            self.attributes
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        }

        fn classes(&self) -> Vec<&str> {
            self.attribute("class")
                .map(|class| class.split_whitespace().collect())
                .unwrap_or_default()
        }
    }

    /// Every start tag in `html`, with its attributes. Dioxus SSR writes text values in double
    /// quotes with the quote character escaped, so a quote always ends such a value, and writes
    /// boolean values bare (`required=true`).
    fn tags(html: &str) -> Vec<Tag> {
        let mut tags = Vec::new();
        let mut rest = html;
        while let Some(start) = rest.find('<') {
            rest = &rest[start + 1..];
            if rest.starts_with('/') || rest.starts_with('!') {
                continue;
            }
            let end = rest.find('>').expect("a start tag should close");
            let (tag, after) = rest.split_at(end);
            rest = &after[1..];
            let tag = tag.trim_end_matches('/');
            let mut parts = tag.splitn(2, char::is_whitespace);
            let element = parts.next().unwrap_or_default().to_owned();
            let attributes = parts.next().map(attributes).unwrap_or_default();
            tags.push(Tag {
                element,
                attributes,
            });
        }
        tags
    }

    fn attributes(mut source: &str) -> Vec<(String, String)> {
        let mut attributes = Vec::new();
        loop {
            source = source.trim_start();
            if source.is_empty() {
                return attributes;
            }
            let name_end = source
                .find(|character: char| character == '=' || character.is_whitespace())
                .unwrap_or(source.len());
            let name = source[..name_end].to_owned();
            source = &source[name_end..];
            let Some(after_equals) = source.strip_prefix('=') else {
                attributes.push((name, String::new()));
                continue;
            };
            let (value, after_value) = match after_equals.strip_prefix('"') {
                Some(quoted) => {
                    let end = quoted.find('"').expect("a quoted value should close");
                    (&quoted[..end], &quoted[end + 1..])
                }
                None => {
                    let end = after_equals
                        .find(char::is_whitespace)
                        .unwrap_or(after_equals.len());
                    after_equals.split_at(end)
                }
            };
            attributes.push((name, value.to_owned()));
            source = after_value;
        }
    }

    /// Every id in `html`.
    fn ids(html: &str) -> Vec<String> {
        tags(html)
            .iter()
            .filter_map(|tag| tag.attribute("id").map(str::to_owned))
            .collect()
    }

    #[test]
    fn string_number_and_integer_controls_render_as_daisyui_inputs() {
        let rendered = RenderedGallery::mount();

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
        let rendered = RenderedGallery::mount();

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
        let mut rendered = RenderedGallery::mount();

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

        let actions = rendered
            .handle
            .node(rendered.control_identity("/newsletter"))
            .expect("the form should be readable")
            .expect("newsletter should exist")
            .actions();
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
        let rendered = RenderedGallery::mount();

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
        let mut rendered = RenderedGallery::mount();

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

        let actions = rendered
            .handle
            .node(rendered.control_identity("/plan"))
            .expect("the form should be readable")
            .expect("plan should exist")
            .actions();
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
        let rendered = RenderedGallery::mount();

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
        let mut rendered = RenderedGallery::mount();
        let billing = rendered.control_identity("/billing");

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
            .handle
            .node(billing)
            .expect("the form should be readable")
            .expect("billing should exist")
            .actions()
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
        let rendered = RenderedGallery::mount();

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
        let mut rendered = RenderedGallery::mount();

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

        let region = rendered.control_identity("/region");
        let actions = rendered
            .handle
            .node(region)
            .expect("the form should be readable")
            .expect("region should exist")
            .actions();
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
        let rendered = RenderedGallery::mount();

        let secret = rendered.control("/secret");
        assert_eq!(secret.attribute("type"), Some("password"));
        assert_eq!(secret.attribute("value"), Some(""));
        assert_eq!(secret.attribute("placeholder"), Some("Choose replacement"));
        assert!(rendered.html().contains("Replace Secret"));
    }

    #[test]
    fn a_read_only_control_renders_as_output_rather_than_an_editable_input() {
        let rendered = RenderedGallery::mount();

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
        let mut rendered = RenderedGallery::mount();
        // Surface an error so `aria-errormessage` is emitted too.
        let quantity = rendered
            .handle
            .node(rendered.control_identity("/quantity"))
            .expect("the form should be readable")
            .expect("quantity should exist")
            .actions();
        quantity
            .input_text("-")
            .expect("the parse blocker should be recorded");
        rendered.settle();

        let html = rendered.html();
        let ids = ids(&html);
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

        let mut references = 0;
        for tag in tags(&html) {
            for attribute in [
                "aria-describedby",
                "aria-errormessage",
                "aria-labelledby",
                "for",
            ] {
                for id in tag
                    .attribute(attribute)
                    .into_iter()
                    .flat_map(str::split_whitespace)
                {
                    references += 1;
                    assert!(
                        ids.iter().any(|candidate| candidate == id),
                        "{attribute}={id} should resolve"
                    );
                }
            }
        }
        assert!(references > 0);
    }

    #[test]
    fn presence_affordances_render_as_daisyui_buttons_carrying_their_ids() {
        let rendered = RenderedGallery::mount();
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
