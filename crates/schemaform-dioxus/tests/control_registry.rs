//! Native contract tests for control renderer resolution.
//!
//! Each test builds a `ControlRegistry`, binds it through `RenderConfigurationBuilder`, and
//! observes only the bind outcome and which registered renderer was called for which control.
//! The built-in renderer is one registration among others, so resolution has no special case
//! for it.

// The registry takes `Arc<dyn ControlRenderer>`; the capturing renderer holds single-threaded
// Dioxus state, which is the supported browser-CSR shape.
#![allow(clippy::arc_with_non_send_sync)]

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
    sync::Arc,
};

use dioxus::prelude::{Element, Props, rsx, use_hook};
use dioxus_core::VirtualDom;
use schemaform::{
    FormDefinition, JsonPointer, WidgetSymbol,
    definition::{DefinitionNodeId, DefinitionNodeView},
    ui::v1::{Binding, Control, Element as UiElement, Stack, UiSchema},
};
use schemaform_dioxus::{
    BindFinding, BuiltinControlRenderer, ControlMatcher, ControlRegistry, ControlRenderContext,
    ControlRenderer, RenderConfiguration, SchemaForm, render::BUILTIN_CONTROL_PRIORITY, use_form,
};
use serde_json::json;

/// Control names rendered by each named capturing renderer.
type RenderedByRenderer = BTreeMap<&'static str, BTreeSet<String>>;
type Rendered = Rc<RefCell<RenderedByRenderer>>;

struct CapturingRenderer {
    name: &'static str,
    rendered: Rendered,
}

impl ControlRenderer for CapturingRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        self.rendered
            .borrow_mut()
            .entry(self.name)
            .or_default()
            .insert(context.control().name.clone());
        rsx! {}
    }
}

struct EveryControl;

impl ControlMatcher for EveryControl {
    fn matches(&self, _definition: DefinitionNodeView<'_>) -> bool {
        true
    }
}

struct BoundTo(&'static str);

impl ControlMatcher for BoundTo {
    fn matches(&self, definition: DefinitionNodeView<'_>) -> bool {
        definition
            .binding()
            .is_some_and(|binding| binding.as_str() == self.0)
    }
}

type RegistryFactory = Rc<dyn Fn(Rendered) -> ControlRegistry>;
type BindOutcome = Rc<RefCell<Option<Result<(), Vec<BindFinding>>>>>;

#[derive(Clone, Props)]
struct RegistryAppProps {
    definition: FormDefinition,
    registry: RegistryFactory,
    rendered: Rendered,
    outcome: BindOutcome,
}

impl PartialEq for RegistryAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.registry, &other.registry)
            && Rc::ptr_eq(&self.rendered, &other.rendered)
            && Rc::ptr_eq(&self.outcome, &other.outcome)
    }
}

fn registry_app(props: RegistryAppProps) -> Element {
    let form = use_form(
        props.definition.clone(),
        json!({ "name": "Ada", "agree": false }),
    )
    .expect("the registry form should be created");
    let bound = use_hook(move || {
        let bound = RenderConfiguration::builder()
            .controls((props.registry)(props.rendered.clone()))
            .build()
            .bind(&form);
        *props.outcome.borrow_mut() = Some(
            bound
                .as_ref()
                .map(|_| ())
                .map_err(|error| error.findings().cloned().collect()),
        );
        bound.ok()
    });
    rsx! {
        if let Some(bound) = bound {
            // `on_error` is optional; this test has no failures to observe.
            SchemaForm {
                form: bound,
                on_submit: move |_| {},
            }
        }
    }
}

fn data_schema() -> serde_json::Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "agree"],
        "properties": {
            "name": { "type": "string", "title": "Name" },
            "agree": { "type": "boolean", "title": "Agree" }
        }
    })
}

/// Mounts the app once and returns the bind outcome and what each capturing renderer rendered.
fn bind_and_mount(
    definition: FormDefinition,
    registry: impl Fn(Rendered) -> ControlRegistry + 'static,
) -> (Result<(), Vec<BindFinding>>, RenderedByRenderer) {
    let rendered: Rendered = Rc::default();
    let outcome: BindOutcome = Rc::default();
    let mut dom = VirtualDom::new_with_props(
        registry_app,
        RegistryAppProps {
            definition,
            registry: Rc::new(registry),
            rendered: rendered.clone(),
            outcome: outcome.clone(),
        },
    );
    dom.rebuild_in_place();
    let outcome = outcome
        .borrow()
        .clone()
        .expect("the registry app should have attempted to bind");
    (outcome, rendered.borrow().clone())
}

/// The definition node id of the control bound to `binding`, found by walking the definition.
fn control_definition(definition: &FormDefinition, binding: &str) -> DefinitionNodeId {
    let mut pending = vec![definition.root()];
    while let Some(id) = pending.pop() {
        let node = definition
            .node(id)
            .expect("definition children should be valid nodes");
        if node
            .binding()
            .is_some_and(|pointer| pointer.as_str() == binding)
        {
            return id;
        }
        pending.extend(node.children());
    }
    panic!("the definition should contain a control bound to {binding}");
}

fn names(values: &[&str]) -> BTreeSet<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn an_empty_registry_reports_no_matching_renderer_for_every_control() {
    let definition =
        FormDefinition::compile(data_schema()).expect("the data schema should compile");
    let name = control_definition(&definition, "/name");
    let agree = control_definition(&definition, "/agree");

    let (outcome, rendered) = bind_and_mount(definition, |_| ControlRegistry::empty());

    // One finding per control, naming the definition node no renderer accepted; the order follows
    // the definition, which is not the data schema's property order.
    let findings = outcome.expect_err("an empty registry should not bind a form with controls");
    let unmatched = findings
        .iter()
        .map(|finding| match finding {
            BindFinding::NoMatchingRenderer { definition_node } => *definition_node,
            other => panic!("unexpected bind finding {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(unmatched.len(), 2);
    assert!(unmatched.contains(&name));
    assert!(unmatched.contains(&agree));
    assert!(rendered.is_empty());
}

#[test]
fn the_builtin_is_an_ordinary_registration_that_matchers_outrank_or_lose_to_by_priority() {
    let definition =
        FormDefinition::compile(data_schema()).expect("the data schema should compile");

    let (outcome, rendered) = bind_and_mount(definition, |rendered| {
        ControlRegistry::with_builtins()
            .matcher(
                BUILTIN_CONTROL_PRIORITY - 1,
                Arc::new(EveryControl),
                Arc::new(CapturingRenderer {
                    name: "below",
                    rendered: rendered.clone(),
                }),
            )
            .matcher(
                BUILTIN_CONTROL_PRIORITY + 1,
                Arc::new(BoundTo("/agree")),
                Arc::new(CapturingRenderer {
                    name: "above",
                    rendered,
                }),
            )
    });

    assert_eq!(outcome, Ok(()));
    // The renderer above the built-in wins its control; the one below never renders anything,
    // and the built-in keeps `/name` without a capturing renderer seeing it.
    assert_eq!(rendered, BTreeMap::from([("above", names(&["/agree"]))]));
}

#[test]
fn a_matcher_tied_with_the_builtin_priority_is_ambiguous() {
    let definition =
        FormDefinition::compile(data_schema()).expect("the data schema should compile");

    let (outcome, _) = bind_and_mount(definition, |rendered| {
        ControlRegistry::with_builtins().matcher(
            BUILTIN_CONTROL_PRIORITY,
            Arc::new(BoundTo("/name")),
            Arc::new(CapturingRenderer {
                name: "tied",
                rendered,
            }),
        )
    });

    assert_eq!(outcome, Err(vec![BindFinding::AmbiguousMatcher]));
}

#[test]
fn the_builtin_renderer_can_be_registered_for_an_exact_widget() {
    let plain = WidgetSymbol::parse("company:plain").expect("the widget symbol should be valid");
    let definition = FormDefinition::compiler(data_schema())
        .ui_schema(UiSchema::new(UiElement::Stack(Stack::new([
            UiElement::Control(
                Control::new(Binding::root(
                    JsonPointer::parse("/name").expect("the name pointer should be valid"),
                ))
                .widget(plain.clone()),
            ),
            UiElement::Control(Control::new(Binding::root(
                JsonPointer::parse("/agree").expect("the agree pointer should be valid"),
            ))),
        ]))))
        .compile()
        .expect("the widget UI schema should compile");

    let (outcome, rendered) = bind_and_mount(definition, move |rendered| {
        ControlRegistry::empty()
            .widget(plain.clone(), Arc::new(BuiltinControlRenderer))
            .matcher(
                BUILTIN_CONTROL_PRIORITY,
                Arc::new(EveryControl),
                Arc::new(CapturingRenderer {
                    name: "fallback",
                    rendered,
                }),
            )
    });

    assert_eq!(outcome, Ok(()));
    // The exact widget never falls back to matching, so only `/agree` reaches the matcher.
    assert_eq!(rendered, BTreeMap::from([("fallback", names(&["/agree"]))]));
}
