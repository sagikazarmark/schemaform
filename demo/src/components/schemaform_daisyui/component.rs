//! The daisyUI control renderer and the registry that selects it.

use std::{rc::Rc, sync::Arc};

use dioxus::prelude::*;
use dioxus_field::FieldContext;
use dioxus_primitives::dioxus_attributes::attributes;
use schemaform::definition::{DefinitionNodeView, SemanticKind};
use schemaform_dioxus::{
    BuiltinControlRenderer, ControlKind, ControlMatcher, ControlRegistry, ControlRenderContext,
    ControlRenderer, render::BUILTIN_CONTROL_PRIORITY, use_text_edit,
};

use super::mapping::{field_meta_values, is_field_error, use_text_binding};
use crate::components::button::{Button, ButtonSize};
use crate::components::field::{Field, FieldDescription, FieldError, FieldLabel};
use crate::components::input::Input;

/// Matcher priority at which [`controls`] registers [`DaisyuiControlRenderer`].
///
/// Above [`BUILTIN_CONTROL_PRIORITY`] so the daisyUI renderer wins every control it accepts
/// and the built-in keeps the rest.
pub const DAISYUI_CONTROL_PRIORITY: i32 = BUILTIN_CONTROL_PRIORITY + 10;

/// A control registry in which string, number, and integer controls render as daisyUI fields
/// and every other control kind falls back to the built-in renderer.
pub fn controls() -> ControlRegistry {
    ControlRegistry::with_builtins().matcher(
        DAISYUI_CONTROL_PRIORITY,
        Arc::new(TextControls),
        Arc::new(DaisyuiControlRenderer),
    )
}

/// Accepts exactly the definition nodes [`DaisyuiControlRenderer`] presents itself: those the
/// adapter derives a string, number, or integer control kind from.
struct TextControls;

impl ControlMatcher for TextControls {
    fn matches(&self, definition: DefinitionNodeView<'_>) -> bool {
        matches!(
            definition.semantic_kind(),
            Some(SemanticKind::String | SemanticKind::Number | SemanticKind::Integer)
        )
    }
}

/// Renders string, number, and integer controls with the registry's `Field` and `Input`.
///
/// The renderer owns the whole control region: label, input, help, findings, and presence
/// affordances. Should a host register it for a control of another kind, that control is
/// handed to [`BuiltinControlRenderer`] rather than to an editable widget the mapping does not
/// cover.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DaisyuiControlRenderer;

impl ControlRenderer for DaisyuiControlRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        // The kind is definition-stable, so a node always renders the same child component and
        // the hooks inside it are called unconditionally.
        match context.control().kind {
            ControlKind::String | ControlKind::Number | ControlKind::Integer => {
                rsx! { TextControl { context } }
            }
            _ => BuiltinControlRenderer.render(context),
        }
    }
}

/// The `inputmode` hint for a text control kind, as the built-in emits it.
fn input_mode(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::Number => "decimal",
        ControlKind::Integer => "numeric",
        _ => "text",
    }
}

/// The kind's name for the `data-schemaform-control` marker the built-in also emits.
fn kind_name(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::Number => "number",
        ControlKind::Integer => "integer",
        _ => "string",
    }
}

/// One daisyUI text control: the renderer's hook-safe child component.
///
/// `Field` receives a fresh context on every render whose binding compares equal across renders
/// (its identity is the edit's hook-stable handles) and whose metadata values it syncs itself,
/// so the registry parts re-render only when the node's presentation actually changes.
///
/// A read-only node renders as noninteractive `output` rather than an `Input` that merely
/// rejects edits, as the built-in does; the facets' `read_only` also covers a node the core will
/// not accept text for right now, which keeps its editable widget and its replace affordance.
#[component]
fn TextControl(context: ControlRenderContext) -> Element {
    let edit = use_text_edit(&context);
    let binding = use_text_binding(edit);
    let Some(projection) = context.node().read().ok().flatten() else {
        return rsx! {};
    };
    let presentation = context.presentation();
    let control = context.control();
    let field_context =
        FieldContext::new(binding).with_meta_values(field_meta_values(presentation, control));

    let element_id = presentation.element_id.clone();
    let kind = kind_name(control.kind);
    let label_class = if presentation.label_visible {
        ""
    } else {
        "sr-only"
    };
    let help = presentation.help.clone();
    let presence = presentation.presence.clone();

    if projection.read_only {
        // Nothing edits this node, so every finding is a description of the shown value and the
        // output carries the adapter's own `aria-describedby`.
        let label = presentation.label.clone();
        let findings = presentation.findings.clone();
        return rsx! {
            Field { context: field_context, "data-schemaform-daisyui": kind,
                FieldLabel { id: format!("{element_id}-label"), class: label_class, "{label}" }
                output {
                    id: element_id.clone(),
                    name: control.name.clone(),
                    class: "min-w-0 py-2",
                    tabindex: "-1",
                    "data-read-only": "",
                    "data-schemaform-control": kind,
                    "aria-invalid": presentation.invalid,
                    "aria-describedby": presentation.described_by(),
                    "{edit.value}"
                }
                if let Some(help) = help {
                    FieldDescription { id: Rc::from(help.id.as_str()), "{help.text}" }
                }
                for finding in findings {
                    FieldDescription {
                        key: "{finding.stable_id}",
                        id: Rc::from(finding.stable_id.as_str()),
                        class: if finding.blocking { "text-error" } else { "text-warning" },
                        "{finding.text}"
                    }
                }
            }
        };
    }

    // An editable write-only widget is labelled by its replacement action, as the built-in does,
    // because the value it holds must not be described.
    let label = control
        .write_only_replacement
        .as_ref()
        .map(|replacement| replacement.label.clone())
        .unwrap_or_else(|| presentation.label.clone());
    let placeholder = control
        .write_only_replacement
        .as_ref()
        .map(|replacement| replacement.placeholder.clone());
    // `FieldError` presents the field errors; the remaining findings are presented as further
    // descriptions so every stable id still resolves to an element.
    let descriptions = presentation
        .findings
        .iter()
        .filter(|finding| !is_field_error(finding))
        .cloned()
        .collect::<Vec<_>>();

    // Listeners cannot travel through `extends`, so the composition events reach the native
    // input through the widget's explicit attribute list together with its other attributes.
    let input_attributes = attributes!(input {
        r#type: if control.write_only { "password" } else { "text" },
        inputmode: input_mode(control.kind),
        readonly: edit.read_only,
        placeholder,
        "data-schemaform-control": kind,
        oncompositionstart: move |_| edit.composition_start.call(()),
        oncompositionend: move |_| edit.composition_end.call(()),
    });

    rsx! {
        Field { context: field_context, "data-schemaform-daisyui": kind,
            FieldLabel { id: format!("{element_id}-label"), class: label_class, "{label}" }
            Input { attributes: input_attributes }
            if let Some(help) = help {
                FieldDescription { id: Rc::from(help.id.as_str()), "{help.text}" }
            }
            for finding in descriptions {
                FieldDescription {
                    key: "{finding.stable_id}",
                    id: Rc::from(finding.stable_id.as_str()),
                    class: if finding.blocking { "text-error" } else { "text-warning" },
                    "{finding.text}"
                }
            }
            FieldError { id: Rc::from(format!("{element_id}-errors")) }
            if !presence.is_empty() {
                div { class: "flex flex-wrap gap-2",
                    for affordance in presence {
                        Button {
                            key: "{affordance.id}",
                            id: affordance.id.clone(),
                            r#type: "button",
                            size: ButtonSize::Sm,
                            onclick: move |_| affordance.invoke.call(()),
                            "{affordance.label}"
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use dioxus::core::{NoOpMutations, VirtualDom};
    use dioxus::prelude::*;
    use schemaform::{FormDefinition, InstanceIdentity};
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

    fn gallery_app(props: GalleryAppProps) -> Element {
        let definition = use_hook(|| {
            FormDefinition::compile(json!({
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
                    "plan": { "title": "Plan", "enum": ["starter", "team"] },
                    "tier": { "title": "Tier", "const": "standard" }
                }
            }))
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
                "plan": "team",
                "tier": "standard"
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
    fn every_other_control_kind_falls_back_to_the_built_in() {
        let rendered = RenderedGallery::mount();

        let boolean = rendered.control("/active");
        assert_eq!(boolean.attribute("type"), Some("checkbox"));
        assert!(!boolean.classes().contains(&"input"));

        let choice = rendered.control("/plan");
        assert_eq!(choice.element, "select");

        let constant = rendered.control("/tier");
        assert_eq!(constant.element, "output");
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
