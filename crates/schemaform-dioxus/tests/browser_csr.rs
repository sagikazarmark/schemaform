#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use dioxus::prelude::*;
use schemaform::{
    CompilationProfile, ExtensionNamespace, ExternalFinding, ExternalFindingBatch, FormDefinition,
    InstanceIdentity, JsonPointer, RetrievalUri, SchemaResource, SubmissionOutcome,
    SubmissionSnapshot, WidgetSymbol,
    definition::DefinitionNodeView,
    form::{ParseBlockerKind, SubmissionBlocker},
    json::parse_ui_schema_v1,
    ui::v1::{
        Auto, Binding, Control, Element as UiElement, ElementMeta, Grid, GridCell, GridSpan, Group,
        PropertyPosition, PropertySelection, Stack, TabPanel, Tabs, Text, TextReference,
        TextSetting, UiSchema,
    },
};
use schemaform_dioxus::{
    CollectionContext, CollectionItemContext, CollectionRenderer, ControlMatcher, ControlRegistry,
    ControlRenderContext, ControlRenderer, ExtensionHandler, ExtensionOccurrence,
    ExtensionPrepareError, ExtensionRenderContext, FindingCollectionPresenter, FormHandle,
    HandleError, HandleTransactionError, Localizer, PreparedExtension, RenderConfiguration,
    RenderEvent, RenderNodeKind, RenderObservation, RenderObserver,
    SchemaForm as RequiredSchemaForm, ShellContext, ShellRenderer, StructureRenderers,
    render::{BindFinding, FindingCollectionContext},
    use_choice_edit, use_form, use_text_edit,
};
use serde_json::json;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::wasm_bindgen_test;
use web_sys::{
    CompositionEvent, CompositionEventInit, Event, EventInit, HtmlFormElement, HtmlIFrameElement,
    HtmlInputElement, HtmlSelectElement, InputEvent, InputEventInit, KeyboardEvent,
    KeyboardEventInit,
};

wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

#[derive(Clone, PartialEq, Props)]
struct SchemaFormProps {
    form: schemaform_dioxus::BoundForm,
    on_submit: EventHandler<SubmissionSnapshot>,
    #[props(default)]
    on_error: Option<EventHandler<HandleError>>,
}

#[allow(non_snake_case)]
fn SchemaForm(props: SchemaFormProps) -> Element {
    if let Some(on_error) = props.on_error {
        return rsx! {
            RequiredSchemaForm {
                form: props.form,
                on_submit: props.on_submit,
                on_error,
            }
        };
    }
    rsx! {
        RequiredSchemaForm {
            form: props.form,
            on_submit: props.on_submit,
            on_error: move |error| {
                assert!(false, "unexpected adapter operation error: {error}");
            },
        }
    }
}

#[path = "../../../testing/fixtures/business-schemas/product_cases.rs"]
mod product_cases;

#[derive(Clone, Props)]
struct TestAppProps {
    handle: Rc<RefCell<Option<FormHandle>>>,
    submitted: Rc<RefCell<Option<SubmissionSnapshot>>>,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl PartialEq for TestAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.handle, &other.handle)
            && Rc::ptr_eq(&self.submitted, &other.submitted)
            && Rc::ptr_eq(&self.errors, &other.errors)
    }
}

#[derive(Clone, Props)]
struct BusinessCorpusAppProps {
    handles: Rc<RefCell<HashMap<String, FormHandle>>>,
    submitted: Rc<RefCell<HashSet<String>>>,
}

impl PartialEq for BusinessCorpusAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.handles, &other.handles) && Rc::ptr_eq(&self.submitted, &other.submitted)
    }
}

#[derive(Clone, Props)]
struct BusinessFixtureProps {
    id: String,
    handles: Rc<RefCell<HashMap<String, FormHandle>>>,
    submitted: Rc<RefCell<HashSet<String>>>,
}

impl PartialEq for BusinessFixtureProps {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && Rc::ptr_eq(&self.handles, &other.handles)
            && Rc::ptr_eq(&self.submitted, &other.submitted)
    }
}

#[derive(Clone, Props)]
struct IsolatedUpdatesTestAppProps {
    handle: Rc<RefCell<Option<FormHandle>>>,
    lifecycle: Rc<RefCell<HashMap<InstanceIdentity, LifecycleCounts>>>,
    matcher_calls: Rc<RefCell<usize>>,
}

#[derive(Clone, Props)]
struct ProductionReactivityTestAppProps {
    handle: Rc<RefCell<Option<FormHandle>>>,
    observations: Rc<RefCell<Vec<RenderObservation>>>,
    mounted: Rc<RefCell<Option<Signal<bool>>>>,
    scenario: browser_workload_pack::Scenario,
}

impl PartialEq for ProductionReactivityTestAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.handle, &other.handle)
            && Rc::ptr_eq(&self.observations, &other.observations)
            && Rc::ptr_eq(&self.mounted, &other.mounted)
            && self.scenario.id == other.scenario.id
    }
}

struct TestRenderObserver {
    observations: Rc<RefCell<Vec<RenderObservation>>>,
}

impl RenderObserver for TestRenderObserver {
    fn observe(&self, observation: RenderObservation) {
        self.observations.borrow_mut().push(observation);
    }
}

impl PartialEq for IsolatedUpdatesTestAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.handle, &other.handle)
            && Rc::ptr_eq(&self.lifecycle, &other.lifecycle)
            && Rc::ptr_eq(&self.matcher_calls, &other.matcher_calls)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct LifecycleCounts {
    renderer_calls: usize,
    mounts: usize,
}

struct InstrumentedRenderer {
    lifecycle: Rc<RefCell<HashMap<InstanceIdentity, LifecycleCounts>>>,
}

impl ControlRenderer for InstrumentedRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        self.lifecycle
            .borrow_mut()
            .entry(context.node().identity())
            .or_default()
            .renderer_calls += 1;
        rsx! {
            InstrumentedControl {
                context,
                lifecycle: self.lifecycle.clone(),
            }
        }
    }
}

struct CountingMatcher {
    calls: Rc<RefCell<usize>>,
}

struct BindingMatcher {
    binding: &'static str,
    calls: Rc<RefCell<usize>>,
}

impl ControlMatcher for BindingMatcher {
    fn matches(&self, definition: DefinitionNodeView<'_>) -> bool {
        *self.calls.borrow_mut() += 1;
        definition
            .binding()
            .is_some_and(|binding| binding.as_str() == self.binding)
    }
}

struct PriorityRenderer {
    marker: &'static str,
    matcher_calls: Rc<RefCell<usize>>,
}

impl ControlRenderer for PriorityRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        let projection = context
            .node()
            .read()
            .expect("the priority-selected control should remain readable")
            .expect("the priority-selected control should remain present");
        let actions = context.actions().clone();
        let presentation = context.presentation();
        let matcher_calls = *self.matcher_calls.borrow();
        rsx! {
            input {
                id: presentation.element_id.clone(),
                name: context.control().name.clone(),
                value: projection.value.unwrap_or_default(),
                "aria-describedby": presentation.described_by(),
                "data-priority-renderer": self.marker,
                "data-matcher-calls": matcher_calls,
                oninput: move |event| {
                    let _ = actions.input_text(event.value());
                },
            }
            {presentation.present_help()}
            {presentation.present_findings()}
        }
    }
}

impl ControlMatcher for CountingMatcher {
    fn matches(&self, _definition: DefinitionNodeView<'_>) -> bool {
        *self.calls.borrow_mut() += 1;
        true
    }
}

/// Matches every control without counting, for apps that hand every control to one renderer.
struct EveryControl;

impl ControlMatcher for EveryControl {
    fn matches(&self, _definition: DefinitionNodeView<'_>) -> bool {
        true
    }
}

/// A custom renderer that owns its whole region: label, input, help, findings, and the presence
/// affordances the adapter computed, each as a button carrying the affordance id and a
/// `data-affordance` kind marker. Every operation result is passed through `report()`.
struct AffordanceRenderer;

impl ControlRenderer for AffordanceRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        let projection = context
            .node()
            .read()
            .expect("the affordance control should remain readable")
            .expect("the affordance control should remain present");
        let actions = context.actions().clone();
        let reporter = context.clone();
        let presentation = context.presentation();
        let control = context.control();
        let presence = presentation.presence.clone();
        rsx! {
            label {
                r#for: presentation.element_id.clone(),
                "{presentation.label}"
            }
            input {
                id: presentation.element_id.clone(),
                name: control.name.clone(),
                value: projection.value.unwrap_or_default(),
                required: control.required,
                disabled: control.disabled,
                readonly: control.read_only,
                "aria-invalid": presentation.invalid,
                "aria-describedby": presentation.described_by(),
                "data-affordance-widget": "",
                oninput: move |event| {
                    reporter.report(actions.input_text(event.value()));
                },
            }
            {presentation.present_help()}
            {presentation.present_findings()}
            for affordance in presence {
                button {
                    key: "{affordance.id}",
                    id: affordance.id.clone(),
                    r#type: "button",
                    "data-affordance": format!("{:?}", affordance.kind),
                    onclick: move |_| affordance.invoke.call(()),
                    "{affordance.label}"
                }
            }
        }
    }
}

struct ExactRenderer {
    matcher_calls: Rc<RefCell<usize>>,
}

struct TestExtensionHandler {
    marker: &'static str,
    preparations: Rc<RefCell<Vec<String>>>,
    error: Option<ExtensionPrepareError>,
}

impl ExtensionHandler for TestExtensionHandler {
    fn prepare(
        &self,
        occurrence: ExtensionOccurrence<'_>,
    ) -> Result<Arc<dyn PreparedExtension>, ExtensionPrepareError> {
        self.preparations
            .borrow_mut()
            .push(occurrence.namespace.as_str().to_owned());
        if let Some(error) = self.error {
            return Err(error);
        }
        assert!(occurrence.value.get("enabled").is_some());
        Ok(Arc::new(TestPreparedExtension {
            marker: self.marker,
            preparations: self.preparations.clone(),
        }))
    }
}

struct TestPreparedExtension {
    marker: &'static str,
    preparations: Rc<RefCell<Vec<String>>>,
}

impl PreparedExtension for TestPreparedExtension {
    fn decorate(&self, context: ExtensionRenderContext, child: Element) -> Element {
        assert_eq!(context.namespace().as_str().ends_with(self.marker), true);
        let preparation_count = self.preparations.borrow().len();
        rsx! {
            div {
                "data-extension": self.marker,
                "data-preparation-count": preparation_count,
                {child}
            }
        }
    }
}

impl ControlRenderer for ExactRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        let projection = context
            .node()
            .read()
            .expect("the exact custom control should remain readable")
            .expect("the exact custom control should remain present");
        let actions = context.actions().clone();
        let presentation = context.presentation();
        let control = context.control();
        let matcher_calls = *self.matcher_calls.borrow();
        let extension_count = context.extensions().iter().count();
        rsx! {
            label {
                r#for: presentation.element_id.clone(),
                "{presentation.label}"
            }
            input {
                id: presentation.element_id.clone(),
                name: control.name.clone(),
                value: projection.value.unwrap_or_default(),
                required: control.required,
                disabled: control.disabled,
                readonly: control.read_only,
                "aria-invalid": presentation.invalid,
                "aria-describedby": presentation.described_by(),
                "data-exact-widget": "",
                "data-control-kind": format!("{:?}", control.kind),
                "data-label-visible": presentation.label_visible,
                "data-help": presentation.help.as_ref().map(|help| help.text.clone()),
                "data-touched": control.touched,
                "data-dirty": control.dirty,
                "data-matcher-calls": matcher_calls,
                "data-extension-count": extension_count,
                oninput: move |event| {
                    let _ = actions.input_text(event.value());
                },
            }
            if let Some(help) = &presentation.help {
                p { id: help.id.clone(), "data-exact-help": "", "{help.text}" }
            }
            {presentation.present_findings()}
        }
    }
}

#[derive(Clone, Props)]
struct InstrumentedControlProps {
    context: ControlRenderContext,
    lifecycle: Rc<RefCell<HashMap<InstanceIdentity, LifecycleCounts>>>,
}

impl PartialEq for InstrumentedControlProps {
    fn eq(&self, other: &Self) -> bool {
        self.context == other.context && Rc::ptr_eq(&self.lifecycle, &other.lifecycle)
    }
}

#[allow(non_snake_case)]
fn InstrumentedControl(props: InstrumentedControlProps) -> Element {
    let identity = props.context.node().identity();
    let lifecycle = props.lifecycle.clone();
    use_hook(move || {
        lifecycle.borrow_mut().entry(identity).or_default().mounts += 1;
    });
    let projection = props
        .context
        .node()
        .read()
        .expect("the instrumented control should remain readable")
        .expect("the instrumented control should remain present");
    let actions = props.context.actions().clone();
    let presentation = props.context.presentation();
    let control = props.context.control();

    rsx! {
        div {
            class: "schemaform-control",
            label {
                r#for: presentation.element_id.clone(),
                "{presentation.label}"
            }
            input {
                id: presentation.element_id.clone(),
                name: control.name.clone(),
                r#type: "text",
                value: projection.value.unwrap_or_default(),
                required: control.required,
                "aria-invalid": presentation.invalid,
                "aria-describedby": presentation.described_by(),
                oninput: move |event| {
                    let _ = actions.input_text(event.value());
                },
            }
            {presentation.present_help()}
            {presentation.present_findings()}
        }
    }
}

fn string_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": {
                "type": "string",
                "title": "Full name"
            }
        }
    }))
    .expect("the trusted data schema should compile");
    let form = use_form(definition.clone(), json!({ "name": "Ada" }))
        .expect("the browser form should be created");
    let second_form = use_form(definition, json!({ "name": "Lin" }))
        .expect("the second browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the generated form should bind to built-in renderers");
    let second_bound = RenderConfiguration::default()
        .bind(&second_form)
        .expect("the second generated form should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let errors = props.errors.clone();

    rsx! {
        div {
            SchemaForm {
                form: bound,
                on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
                on_error: move |error| errors.borrow_mut().push(error),
            }
            SchemaForm {
                form: second_bound,
                on_submit: move |_| {},
            }
        }
    }
}

fn business_corpus_test_app(props: BusinessCorpusAppProps) -> Element {
    let fixtures = product_cases::fixtures()
        .into_iter()
        .filter(product_cases::BusinessSchemaFixture::is_in_profile)
        .collect::<Vec<_>>();
    rsx! {
        div {
            for fixture in fixtures {
                BusinessFixture {
                    key: "{fixture.id}",
                    id: fixture.id,
                    handles: props.handles.clone(),
                    submitted: props.submitted.clone(),
                }
            }
        }
    }
}

#[allow(non_snake_case)]
fn BusinessFixture(props: BusinessFixtureProps) -> Element {
    let fixture = product_cases::fixtures()
        .into_iter()
        .find(|fixture| fixture.id == props.id)
        .expect("the business fixture should remain embedded");
    let initial_form_data = fixture
        .expected_controls
        .iter()
        .filter(|control| {
            control.kind == "homogeneous-array" && !control.binding[1..].contains('/')
        })
        .fold(serde_json::Map::new(), |mut data, control| {
            data.insert(control.binding[1..].to_owned(), json!([]));
            data
        });
    let definition = fixture.compiler().compile().unwrap_or_else(|error| {
        panic!("in-profile fixture {} should compile: {error}", fixture.id)
    });
    let form = use_form(definition, initial_form_data.into()).unwrap_or_else(|error| {
        panic!("in-profile fixture {} should execute: {error}", fixture.id)
    });
    let bound = RenderConfiguration::default()
        .bind(&form)
        .unwrap_or_else(|error| {
            panic!(
                "in-profile fixture {} should bind to default renderers: {error}",
                fixture.id
            )
        });
    props
        .handles
        .borrow_mut()
        .entry(fixture.id.clone())
        .or_insert_with(|| form.clone());
    let submitted = props.submitted.clone();
    let id = fixture.id.clone();
    rsx! {
        article { "data-business-schema-fixture": fixture.id,
            SchemaForm {
                form: bound,
                on_submit: move |_| {
                    submitted.borrow_mut().insert(id.clone());
                },
            }
        }
    }
}

fn authored_ui_test_app(props: TestAppProps) -> Element {
    let pointer = |value| JsonPointer::parse(value).expect("the test binding should be valid");
    let ui_schema = UiSchema::new(UiElement::Stack(Stack::new([
        UiElement::Text(Text::new(TextReference::localized(
            "profile.intro",
            "Use <strong>plain text</strong>.",
        ))),
        UiElement::Control(
            Control::new(Binding::root(pointer("/second")))
                .label(TextSetting::Value(TextReference::localized(
                    "profile.second",
                    "Second field",
                )))
                .help(TextSetting::Value(TextReference::localized(
                    "profile.second.help",
                    "Enter the family name.",
                ))),
        ),
        UiElement::Group(Group::new(
            TextReference::localized("profile.primary", "Primary details"),
            UiElement::Control(Control::new(Binding::root(pointer("/first")))),
        )),
    ])));
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["first", "second", "hidden"],
        "properties": {
            "first": { "type": "string", "title": "First field" },
            "second": { "type": "string", "minLength": 1 },
            "hidden": { "type": "string" }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored UI schema should compile");
    let form = use_form(
        definition,
        json!({ "first": "Ada", "second": "Lovelace", "hidden": "preserved" }),
    )
    .expect("the authored browser form should be created");
    let bound = RenderConfiguration::builder()
        .localizer(Arc::new(AuthoredTestLocalizer))
        .build()
        .bind(&form)
        .expect("the authored form should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn tabs_test_app(props: TestAppProps) -> Element {
    let pointer = |value| JsonPointer::parse(value).expect("the test binding should be valid");
    let ui_schema = UiSchema::new(UiElement::Tabs(Tabs::new([
        TabPanel::new(
            TextReference::literal("Account"),
            UiElement::Control(Control::new(Binding::root(pointer("/name")))),
        ),
        TabPanel::new(
            TextReference::literal("Contact"),
            UiElement::Control(Control::new(Binding::root(pointer("/email")))),
        ),
    ])));
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["name", "email"],
        "properties": {
            "name": { "type": "string", "title": "Name" },
            "email": { "type": "string", "title": "Email", "minLength": 3 }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored tabs should compile");
    let form = use_form(definition, json!({ "name": "Ada", "email": "Li" }))
        .expect("the tabs form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the authored tabs should bind");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn responsive_grid_test_app(props: TestAppProps) -> Element {
    let pointer = |value| JsonPointer::parse(value).expect("the test binding should be valid");
    let span = |value| GridSpan::new(value).expect("the test grid span should be valid");
    let ui_schema = UiSchema::new(UiElement::Grid(Grid::new([
        GridCell::new(
            span(12),
            UiElement::Control(Control::new(Binding::root(pointer("/first")))),
        )
        .wide_span(span(4)),
        GridCell::new(
            span(12),
            UiElement::Control(Control::new(Binding::root(pointer("/second")))),
        )
        .wide_span(span(8)),
    ])));
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["first", "second"],
        "properties": {
            "first": { "type": "string", "title": "First" },
            "second": { "type": "string", "title": "Second", "minLength": 3 }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the responsive grid should compile");
    let form = use_form(definition, json!({ "first": "Ada", "second": "Byron" }))
        .expect("the responsive grid form should be created");
    let bound = RenderConfiguration::builder()
        .grid_wide_breakpoint_css_px(640)
        .build()
        .bind(&form)
        .expect("the responsive grid should bind");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn auto_region_test_app(props: TestAppProps) -> Element {
    let pointer = |value| JsonPointer::parse(value).expect("the test binding should be valid");
    let ui_schema = UiSchema::new(UiElement::Stack(Stack::new([
        UiElement::Text(Text::new(TextReference::literal("Generated fields"))),
        UiElement::Auto(
            Auto::new(Binding::root(pointer(""))).properties(
                PropertySelection::default()
                    .include("first")
                    .include("second")
                    .order([
                        PropertyPosition::Property("second".to_owned()),
                        PropertyPosition::Property("first".to_owned()),
                    ]),
            ),
        ),
        UiElement::Text(Text::new(TextReference::literal("End fields"))),
    ])));
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "first": { "type": "string" },
            "second": { "type": "string" },
            "hidden": { "type": "string" }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored Auto region should compile");
    let form = use_form(
        definition,
        json!({ "first": "Ada", "second": "Lovelace", "hidden": "preserved" }),
    )
    .expect("the authored Auto browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the generated region should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

struct AuthoredTestLocalizer;

impl Localizer for AuthoredTestLocalizer {
    fn localize(&self, message: &schemaform_dioxus::render::MessageDescriptor) -> String {
        match message.key.as_deref() {
            Some("profile.intro") => "Use <strong>localized plain text</strong>.".to_owned(),
            Some("profile.second") => "Localized second field".to_owned(),
            Some("profile.second.help") => "Localized family-name help.".to_owned(),
            Some("profile.primary") => "Localized primary details".to_owned(),
            Some("array.entry") => "Entry".to_owned(),
            _ => message.fallback.clone(),
        }
    }
}

struct DescriptorPresenter;

impl FindingCollectionPresenter for DescriptorPresenter {
    fn render(&self, context: FindingCollectionContext) -> Element {
        let summary = context.is_summary();
        let findings = context.findings().cloned().collect::<Vec<_>>();
        let finding_count = findings.len();
        rsx! {
            div {
                "data-descriptor-collection": if summary { "summary" } else { "local" },
                "data-descriptor-count": finding_count,
                for finding in findings {
                    div {
                        id: finding.stable_id,
                        "data-descriptor-code": finding.code,
                        "data-descriptor-kind": format!("{:?}", finding.kind),
                        "data-descriptor-blocking": finding.blocking.to_string(),
                        "data-descriptor-parameters": finding.parameters.to_string(),
                        "{finding.text}"
                    }
                }
            }
        }
    }
}

struct ReactiveTestLocalizer {
    alternate: Signal<bool>,
}

struct ImeTestLocalizer {
    alternate: Signal<bool>,
}

impl Localizer for ImeTestLocalizer {
    fn localize(&self, message: &schemaform_dioxus::render::MessageDescriptor) -> String {
        if *self.alternate.read() && message.key.as_deref() == Some("ime.quantity") {
            "Localized quantity".to_owned()
        } else {
            message.fallback.clone()
        }
    }
}

fn ime_test_app(props: TestAppProps) -> Element {
    let mut alternate = use_signal(|| false);
    let ui_schema = UiSchema::new(UiElement::Control(
        Control::new(Binding::root(JsonPointer::parse("/quantity").unwrap())).label(
            TextSetting::Value(TextReference::localized("ime.quantity", "Quantity")),
        ),
    ));
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["quantity"],
        "properties": { "quantity": { "type": "integer" } }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the IME test definition should compile");
    let form = use_form(definition, json!({ "quantity": 1 }))
        .expect("the IME test form should be created");
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .localizer(Arc::new(ImeTestLocalizer { alternate }))
            .build()
            .bind(&bound_form)
            .expect("the IME test form should bind")
    });
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let errors = props.errors.clone();

    rsx! {
        button {
            r#type: "button",
            "data-ime-change-locale": "",
            onclick: move |_| alternate.toggle(),
            "Change locale"
        }
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
            on_error: move |error| errors.borrow_mut().push(error),
        }
    }
}

fn operation_error_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["text", "enabled", "choice", "items"],
        "properties": {
            "text": { "type": "string", "title": "Text" },
            "optional": { "type": ["string", "null"], "title": "Optional" },
            "enabled": { "type": "boolean", "title": "Enabled" },
            "choice": { "enum": ["a", "b"], "title": "Choice" },
            "items": {
                "type": "array",
                "title": "Items",
                "maxItems": 4,
                "items": { "type": "string", "title": "Item" }
            }
        }
    }))
    .expect("the operation-error data schema should compile");
    let form = use_form(
        definition,
        json!({ "text": "canonical", "enabled": false, "choice": "a", "items": ["one", "two"] }),
    )
    .expect("the operation-error form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the operation-error form should bind");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let errors = props.errors.clone();

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
            on_error: move |error| errors.borrow_mut().push(error),
        }
    }
}

/// Binds every control to [`AffordanceRenderer`] above the built-in priority.
fn bind_affordance_renderer(form: &FormHandle) -> schemaform_dioxus::BoundForm {
    RenderConfiguration::builder()
        .controls(ControlRegistry::with_builtins().matcher(
            schemaform_dioxus::render::BUILTIN_CONTROL_PRIORITY + 10,
            Arc::new(EveryControl),
            Arc::new(AffordanceRenderer),
        ))
        .build()
        .bind(form)
        .expect("the affordance renderer should bind every control")
}

fn custom_renderer_operation_error_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["text"],
        "properties": {
            "text": { "type": "string", "title": "Text" },
            "optional": { "type": ["string", "null"], "title": "Optional" }
        }
    }))
    .expect("the custom-renderer operation-error data schema should compile");
    let form = use_form(definition, json!({ "text": "canonical" }))
        .expect("the custom-renderer operation-error form should be created");
    let bound = bind_affordance_renderer(&form);
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let errors = props.errors.clone();

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
            on_error: move |error| errors.borrow_mut().push(error),
        }
    }
}

/// A form shell with none of the built-in's chrome: the body first, the summary framed as an
/// aside after it, and the submit affordance as a plain button that invokes the affordance rather
/// than a `type="submit"` button. Submission and summary focus therefore have to come from the
/// adapter's contract alone.
struct TestShell;

impl ShellRenderer for TestShell {
    fn shell(&self, context: ShellContext) -> Element {
        let submit = context.submit;
        rsx! {
            section { "data-test-shell": "body", {context.body} }
            aside { "data-test-shell": "summary", {context.summary} }
            footer {
                "data-test-shell": "footer",
                button {
                    id: submit.id.clone(),
                    r#type: "button",
                    "data-test-shell-submit": "",
                    "data-affordance": format!("{:?}", submit.kind),
                    onclick: move |_| submit.invoke.call(()),
                    "{submit.label}"
                }
            }
        }
    }
}

fn custom_shell_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "title": "Full name", "minLength": 2 }
        }
    }))
    .expect("the custom-shell data schema should compile");
    let form = use_form(definition, json!({ "name": "Ada" }))
        .expect("the custom-shell form should be created");
    let bound = RenderConfiguration::builder()
        .structure(StructureRenderers::default().with_shell(TestShell))
        .build()
        .bind(&form)
        .expect("the built-in control should bind under a custom shell");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let errors = props.errors.clone();

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
            on_error: move |error| errors.borrow_mut().push(error),
        }
    }
}

/// A collection renderer with none of the built-in's chrome: a `section` instead of a
/// `fieldset`, a heading instead of a legend, an explicit empty state, the live region wrapped in
/// a visually hidden element, and item actions rendered *before* the item's children. Buttons
/// carry only their affordance id and accessible name, never the built-in's `data-*` markers, so
/// identity, focus and announcements have to come from the adapter's contract alone.
struct TestCollection;

fn test_affordance_button(affordance: schemaform_dioxus::Affordance) -> Element {
    rsx! {
        button {
            key: "{affordance.id}",
            id: affordance.id.clone(),
            r#type: "button",
            "data-test-affordance": format!("{:?}", affordance.kind),
            "aria-label": affordance.accessible_name.clone(),
            onclick: move |_| affordance.invoke.call(()),
            "{affordance.label}"
        }
    }
}

impl CollectionRenderer for TestCollection {
    fn collection(&self, context: CollectionContext) -> Element {
        let presentation = context.presentation;
        let element_id = presentation.element_id.clone();
        let described_by = presentation.described_by();
        let help = presentation.present_help();
        let findings = presentation.present_findings();
        let presence = presentation.presence.clone();
        let incompatible_value = presentation.incompatible_value.clone();
        rsx! {
            section {
                id: element_id.clone(),
                "data-test-collection": "",
                "data-test-count": "{context.count}",
                tabindex: "-1",
                "aria-labelledby": "{element_id}-title",
                "aria-invalid": presentation.invalid,
                "aria-describedby": described_by,
                h2 { id: "{element_id}-title", "{presentation.label}" }
                {help}
                div { "data-test-collection-presence": "",
                    if let Some(value) = incompatible_value {
                        output { "data-test-incompatible": "", "{value}" }
                    }
                    for affordance in presence {
                        {test_affordance_button(affordance)}
                    }
                }
                div { class: "visually-hidden", {context.announcement} }
                if context.count == 0 {
                    p { "data-test-empty": "", "{context.item_label}: none" }
                }
                ol { "data-test-items": "", {context.items} }
                if let Some(append) = context.append {
                    {test_affordance_button(append)}
                }
                {findings}
            }
        }
    }

    fn collection_item(&self, context: CollectionItemContext) -> Element {
        let actions = [
            context.move_up,
            context.move_down,
            context.insert_before,
            context.remove,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        let title_id = format!("{}-title", context.row_id);
        rsx! {
            li { "data-test-item": "", "aria-labelledby": title_id.clone(),
                header {
                    span { id: title_id, "{context.item_label} {context.position}/{context.count}" }
                    for affordance in actions {
                        {test_affordance_button(affordance)}
                    }
                }
                {context.children}
            }
        }
    }
}

fn custom_collection_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "tags": {
                "type": "array",
                "title": "Tags",
                "default": ["seed"],
                "maxItems": 3,
                "items": {
                    "type": "string",
                    "title": "Tag",
                    "default": "valid",
                    "minLength": 4
                }
            }
        }
    }))
    .expect("the custom-collection data schema should compile");
    let form = use_form(definition, json!({ "tags": ["same", "same"] }))
        .expect("the custom-collection form should be created");
    let bound = RenderConfiguration::builder()
        .structure(StructureRenderers::default().with_collection(TestCollection))
        .build()
        .bind(&form)
        .expect("the built-in item control should bind under a custom collection");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let errors = props.errors.clone();

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
            on_error: move |error| errors.borrow_mut().push(error),
        }
    }
}

fn custom_renderer_presence_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "value": { "type": ["string", "null"], "title": "Value" },
            "note": { "type": "string", "title": "Note", "description": "Optional note" }
        }
    }))
    .expect("the custom-renderer presence data schema should compile");
    let form = use_form(definition, json!({}))
        .expect("the custom-renderer presence form should be created");
    let bound = bind_affordance_renderer(&form);
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

impl Localizer for ReactiveTestLocalizer {
    fn localize(&self, message: &schemaform_dioxus::render::MessageDescriptor) -> String {
        if !*self.alternate.read() {
            return message.fallback.clone();
        }
        match message.key.as_deref() {
            Some("profile.reactive-intro") => "Localized <strong>intro</strong>.".to_owned(),
            Some("schemaform.validation.minimum") => format!(
                "Localized minimum <strong>{}</strong>.",
                message
                    .parameters
                    .get("limit")
                    .expect("minimum localization should receive its structured limit")
            ),
            Some("schemaform.array.added.announcement") => format!(
                "Localized item added at position {}.",
                message
                    .parameters
                    .get("position")
                    .expect("array announcements should receive their structured position")
            ),
            _ => format!("Localized {}", message.fallback),
        }
    }
}

struct ReactivePresenter {
    collection: &'static str,
    mode: &'static str,
}

impl FindingCollectionPresenter for ReactivePresenter {
    fn render(&self, context: FindingCollectionContext) -> Element {
        let findings = context
            .entries()
            .map(|entry| (entry.finding().clone(), Some(entry.target_focus().clone())))
            .collect::<Vec<_>>();
        rsx! {
            div {
                "data-reactive-presenter": self.collection,
                "data-presenter-mode": self.mode,
                for (finding, target) in findings {
                    div {
                        id: finding.stable_id,
                        "data-reactive-finding": finding.code,
                        "data-reactive-parameters": finding.parameters.to_string(),
                        if let Some(target) = target {
                            button {
                                r#type: "button",
                                onclick: move |_| target.focus(),
                                "{finding.text}"
                            }
                        } else {
                            "{finding.text}"
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct ReactivePresentationServices {
    localizer: Arc<dyn Localizer>,
    local_default: Arc<dyn FindingCollectionPresenter>,
    local_alternate: Arc<dyn FindingCollectionPresenter>,
    summary_default: Arc<dyn FindingCollectionPresenter>,
    summary_alternate: Arc<dyn FindingCollectionPresenter>,
}

struct PresentationRenderer {
    calls: Rc<RefCell<usize>>,
}

impl ControlRenderer for PresentationRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        *self.calls.borrow_mut() += 1;
        let calls = *self.calls.borrow();
        let projection = context
            .node()
            .read()
            .expect("the localized custom control should remain readable")
            .expect("the localized custom control should remain present");
        let actions = context.actions().clone();
        let presentation = context.presentation();
        rsx! {
            div {
                "data-presentation-renderer": "",
                "data-renderer-calls": calls,
                label {
                    r#for: presentation.element_id.clone(),
                    "{presentation.label}"
                }
                input {
                    id: presentation.element_id.clone(),
                    name: context.control().name.clone(),
                    value: projection.value.unwrap_or_default(),
                    "aria-invalid": presentation.invalid,
                    "aria-describedby": presentation.described_by(),
                    oninput: move |event| {
                        let _ = actions.input_text(event.value());
                    },
                }
                {presentation.present_help()}
                {presentation.present_findings()}
            }
        }
    }
}

fn reactive_presentation_test_app(props: TestAppProps) -> Element {
    let mut locale = use_signal(|| false);
    let ui_schema = UiSchema::new(UiElement::Stack(Stack::new([
        UiElement::Text(Text::new(TextReference::localized(
            "profile.reactive-intro",
            "Fallback <strong>intro</strong>.",
        ))),
        UiElement::Auto(Auto::new(Binding::root(
            JsonPointer::parse("").expect("the root binding should be valid"),
        ))),
    ])));
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity", "settings", "tags"],
        "properties": {
            "quantity": {
                "type": "integer",
                "title": "Quantity",
                "minimum": 2
            },
            "settings": {
                "type": "object",
                "title": "Settings",
                "description": "Account settings.",
                "additionalProperties": false,
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "title": "Name" }
                }
            },
            "tags": {
                "type": "array",
                "title": "Tags",
                "items": { "type": "string", "title": "Tag" }
            }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the reactive presentation definition should compile");
    let form = use_form(
        definition,
        json!({ "quantity": 1, "settings": { "name": "Ada" }, "tags": ["one"] }),
    )
    .expect("the reactive presentation form should be created");
    let services = use_hook(move || ReactivePresentationServices {
        localizer: Arc::new(ReactiveTestLocalizer { alternate: locale }),
        local_default: Arc::new(ReactivePresenter {
            collection: "local",
            mode: "default",
        }),
        local_alternate: Arc::new(ReactivePresenter {
            collection: "local",
            mode: "alternate",
        }),
        summary_default: Arc::new(ReactivePresenter {
            collection: "summary",
            mode: "default",
        }),
        summary_alternate: Arc::new(ReactivePresenter {
            collection: "summary",
            mode: "alternate",
        }),
    });
    let renderer_calls = use_hook(|| Rc::new(RefCell::new(0)));
    let bound_form = form.clone();
    let bound_services = services.clone();
    let bound_renderer_calls = renderer_calls.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .controls(ControlRegistry::with_builtins().matcher(
                10,
                Arc::new(BindingMatcher {
                    binding: "/quantity",
                    calls: Rc::new(RefCell::new(0)),
                }),
                Arc::new(PresentationRenderer {
                    calls: bound_renderer_calls,
                }),
            ))
            .local_presenter(bound_services.local_default.clone())
            .summary_presenter(bound_services.summary_default.clone())
            .localizer(bound_services.localizer.clone())
            .build()
            .bind(&bound_form)
            .expect("the reactive presentation services should bind")
    });
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let local_configuration = RenderConfiguration::builder()
        .local_presenter(services.local_alternate.clone())
        .summary_presenter(services.summary_default.clone())
        .localizer(services.localizer.clone())
        .build();
    let local_bound = bound.clone();
    let summary_configuration = RenderConfiguration::builder()
        .local_presenter(services.local_alternate.clone())
        .summary_presenter(services.summary_alternate.clone())
        .localizer(services.localizer.clone())
        .build();
    let summary_bound = bound.clone();

    rsx! {
        button {
            r#type: "button",
            "data-change-locale": "",
            onclick: move |_| locale.toggle(),
            "Change locale"
        }
        button {
            r#type: "button",
            "data-change-local-presenter": "",
            onclick: move |_| local_configuration.rebind_presentation(&local_bound),
            "Change local presenter"
        }
        button {
            r#type: "button",
            "data-change-summary-presenter": "",
            onclick: move |_| summary_configuration.rebind_presentation(&summary_bound),
            "Change summary presenter"
        }
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn presenter_collection_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer",
                "title": "Quantity",
                "minimum": 2
            }
        }
    }))
    .expect("the presenter data schema should compile");
    let form = use_form(definition, json!({ "quantity": 1 }))
        .expect("the presenter form should be created");
    let bound = RenderConfiguration::builder()
        .local_presenter(Arc::new(DescriptorPresenter))
        .summary_presenter(Arc::new(DescriptorPresenter))
        .build()
        .bind(&form)
        .expect("the custom finding presenters should bind");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn annotation_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "email": {
                "type": "string",
                "title": "Email address",
                "description": "Where account notices are sent.",
                "format": "email",
                "default": "default@example.test",
                "deprecated": true,
                "examples": ["example@example.test"],
                "contentEncoding": "base64",
                "contentMediaType": "text/plain",
                "contentSchema": { "type": "string" }
            },
            "alias": {
                "type": "string",
                "title": "Alias",
                "default": "schema seed"
            },
            "nickname": {
                "allOf": [
                    {
                        "type": "string",
                        "title": "Public name",
                        "description": "Shown to other users."
                    },
                    {
                        "title": "Nickname",
                        "description": "A short display name."
                    }
                ]
            }
        }
    }))
    .expect("presentation annotations should compile without strengthening validation");
    let form = use_form(
        definition,
        json!({ "email": "not an email or base64", "nickname": "Ada" }),
    )
    .expect("the annotated browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the annotated controls should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn annotation_authority_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": [
            "profile",
            "secret",
            "secret_count",
            "secret_rate",
            "secret_enabled",
            "secret_mode",
            "secret_region",
            "credentials"
        ],
        "properties": {
            "profile": {
                "type": "object",
                "readOnly": true,
                "additionalProperties": false,
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "title": "Name" }
                }
            },
            "secret": {
                "type": "string",
                "title": "Secret",
                "writeOnly": true
            },
            "secret_count": {
                "type": "integer",
                "title": "Secret count",
                "writeOnly": true
            },
            "secret_rate": {
                "type": "number",
                "title": "Secret rate",
                "writeOnly": true
            },
            "secret_enabled": {
                "type": "boolean",
                "title": "Secret enabled",
                "writeOnly": true
            },
            "secret_mode": {
                "enum": ["private", "public"],
                "title": "Secret mode",
                "writeOnly": true
            },
            "secret_region": {
                "const": "EU",
                "title": "Secret region",
                "writeOnly": true
            },
            "credentials": {
                "type": "object",
                "title": "Credentials",
                "writeOnly": true,
                "additionalProperties": false,
                "required": ["token"],
                "properties": {
                    "token": { "type": "string", "title": "Token" }
                }
            }
        }
    }))
    .expect("read-only and write-only annotations should compile");
    let form = use_form(
        definition,
        json!({
            "profile": { "name": "Ada" },
            "secret": "existing secret",
            "secret_count": 7,
            "secret_rate": 1.5,
            "secret_enabled": false,
            "secret_mode": "private",
            "secret_region": "EU",
            "credentials": { "token": "nested secret" }
        }),
    )
    .expect("the annotated browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the annotated controls should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn scalar_presence_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "value": { "type": ["string", "null"], "title": "Value" },
            "settings": {
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" },
                    "mode": { "enum": ["basic", "advanced"] }
                }
            }
        }
    }))
    .expect("the nullable scalar data schema should compile");
    let form = use_form(definition, json!({})).expect("the scalar presence form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the nullable scalar should bind to a built-in renderer");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn integer_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer",
                "title": "Quantity",
                "minimum": 184467440737095516160_u128
            }
        }
    }))
    .expect("the trusted data schema should compile");
    let baseline_data = serde_json::from_str(r#"{"quantity":184467440737095516160}"#)
        .expect("the arbitrary-precision baseline should parse");
    let form = use_form(definition, baseline_data).expect("the browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the generated form should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn number_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(
        serde_json::from_str(
            r#"{
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": false,
                "required": ["rate"],
                "properties": {
                    "rate": {
                        "type": "number",
                        "title": "Rate",
                        "minimum": 0.1000000000000000000000000000000000000001
                    }
                }
            }"#,
        )
        .expect("the decimal data schema should parse"),
    )
    .expect("the trusted decimal data schema should compile");
    let baseline_data =
        serde_json::from_str(r#"{"rate":0.1000000000000000000000000000000000000001}"#)
            .expect("the arbitrary-precision decimal baseline should parse");
    let form = use_form(definition, baseline_data).expect("the browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the generated form should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn lifecycle_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["quantity"],
        "properties": {
            "quantity": {
                "type": "integer",
                "title": "Quantity",
                "minimum": 1
            }
        }
    }))
    .expect("the lifecycle data schema should compile");
    let baseline = serde_json::from_str(r#"{"quantity":1e3}"#)
        .expect("the arbitrary-precision lifecycle baseline should parse");
    let form = use_form(definition, baseline).expect("the browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the lifecycle form should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn boolean_constant_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["enabled", "region"],
        "properties": {
            "enabled": { "type": "boolean", "title": "Enabled" },
            "region": { "const": "EU", "title": "Region" }
        }
    }))
    .expect("the boolean and constant data schema should compile");
    let form = use_form(definition, json!({ "enabled": false, "region": "EU" }))
        .expect("the browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("boolean and constant controls should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn choice_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(
        serde_json::from_str(
            r#"{
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "required": ["choice", "nothing", "region"],
                "properties": {
                    "choice": {
                        "title": "Choice",
                        "enum": [null, true, "true", 1.0000000000000000000000000000000000000001, "1.0000000000000000000000000000000000000001"]
                    },
                    "nothing": { "type": "null", "title": "Nothing" },
                    "region": { "const": "EU", "title": "Region" }
                }
            }"#,
        )
        .expect("the browser choice schema should parse"),
    )
    .expect("the browser choice schema should compile");
    let form = use_form(
        definition,
        json!({ "choice": null, "nothing": null, "region": "EU" }),
    )
    .expect("the browser choice form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("choice and fixed controls should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn unsupported_one_of_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["contact", "name"],
        "properties": {
            "contact": {
                "title": "Contact",
                "oneOf": [
                    { "type": "string" },
                    { "type": "integer", "minimum": 1 }
                ]
            },
            "name": { "type": "string", "title": "Name" }
        }
    }))
    .analyze()
    .expect("lenient analysis should retain the unsupported region")
    .into_parts()
    .0;
    let form = use_form(
        definition,
        json!({ "contact": "ada@example.test", "name": "Ada" }),
    )
    .expect("the browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the intentional unsupported region should bind");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn open_object_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": true,
        "propertyNames": { "pattern": "^[a-z]+$" },
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "title": "Name" }
        }
    }))
    .expect("the open fixed-object projection should compile");
    let form = use_form(
        definition,
        json!({
            "name": "Ada",
            "hostowned": { "source": "import", "version": 7 }
        }),
    )
    .expect("the browser open-object form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the declared open-object controls should bind");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn constrained_open_object_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": { "type": "integer" },
        "patternProperties": { "^x-": { "minimum": 0 } },
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "title": "Name" }
        }
    }))
    .expect("the constrained fixed projection should compile with warnings");
    let form = use_form(
        definition,
        json!({ "name": "Ada", "x-score": 7, "hostowned": 2 }),
    )
    .expect("the constrained open-object browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the constrained fixed projection should bind");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

#[cfg(schemaform_test_validation_faults)]
fn indeterminate_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "x-schemaform-test-validation-fault": true,
        "type": "object",
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "title": "Name", "minLength": 3 }
        }
    }))
    .expect("the private validator fault fixture should compile");
    let form = use_form(definition, json!({ "name": "" }))
        .expect("the indeterminate browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the indeterminate form should bind");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn nested_fixed_object_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:nested-local-ref-browser",
        "type": "object",
        "additionalProperties": false,
        "required": ["contact"],
        "properties": {
            "contact": {
                "$ref": "#/$defs/contact",
                "properties": {
                    "name": { "minLength": 3 }
                }
            }
        },
        "$defs": {
            "contact": {
                "type": "object",
                "title": "Contact",
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "title": "Name" }
                }
            }
        }
    }))
    .expect("the nested local reference should compile");
    let form = use_form(definition, json!({ "contact": { "name": "Ada" } }))
        .expect("the nested browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the nested form should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn optional_fixed_object_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "settings": {
                "type": "object",
                "title": "Settings",
                "default": { "name": "Li" },
                "additionalProperties": false,
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "title": "Name", "minLength": 3 }
                }
            }
        }
    }))
    .expect("the optional fixed object should compile");
    let form = use_form(definition, json!({}))
        .expect("the absent optional object should remain constructible");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the optional fixed object should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn anchored_resource_graph_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://schemas.example/forms/browser-root.json",
        "type": "object",
        "additionalProperties": false,
        "required": ["contact"],
        "properties": {
            "contact": { "$ref": "../shared/browser-contact.json#contact" }
        }
    }))
    .root_uri(
        RetrievalUri::parse("https://retrieval.example/browser-root.json")
            .expect("the browser root retrieval URI should be valid"),
    )
    .resource(SchemaResource::new(
        RetrievalUri::parse("https://cdn.example/browser-contact.json")
            .expect("the browser resource retrieval URI should be valid"),
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$id": "https://schemas.example/shared/browser-contact.json",
            "$defs": {
                "contact": {
                    "$anchor": "contact",
                    "type": "object",
                    "title": "Contact",
                    "additionalProperties": false,
                    "required": ["name"],
                    "properties": {
                        "name": {
                            "type": "string",
                            "title": "Name",
                            "minLength": 3
                        }
                    }
                }
            }
        }),
    ))
    .compile()
    .expect("the complete browser resource graph should compile");
    let form = use_form(definition, json!({ "contact": { "name": "Ada" } }))
        .expect("the anchored browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the anchored form should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn compatible_all_of_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "urn:schemaform:test:compatible-all-of-browser",
        "type": "object",
        "allOf": [
            {
                "required": ["name"],
                "properties": {
                    "name": { "type": "string", "title": "Name" }
                }
            },
            {
                "properties": {
                    "name": { "minLength": 3 }
                }
            }
        ]
    }))
    .expect("the compatible allOf composition should compile");
    let form = use_form(definition, json!({ "name": "Ada" }))
        .expect("the composed browser form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the composed form should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn isolated_updates_test_app(props: IsolatedUpdatesTestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["first_name", "last_name"],
        "properties": {
            "first_name": { "type": "string", "title": "First name" },
            "last_name": { "type": "string", "title": "Last name" }
        }
    }))
    .expect("the trusted data schema should compile");
    let form = use_form(
        definition,
        json!({ "first_name": "Ada", "last_name": "Lovelace" }),
    )
    .expect("the browser form should be created");
    let renderer = Arc::new(InstrumentedRenderer {
        lifecycle: props.lifecycle.clone(),
    });
    let configuration = RenderConfiguration::builder()
        .controls(ControlRegistry::with_builtins().matcher(
            10,
            Arc::new(CountingMatcher {
                calls: props.matcher_calls.clone(),
            }),
            renderer,
        ))
        .build();
    let bound = configuration
        .bind(&form)
        .expect("the generated form should bind to the instrumented renderer");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |_| {},
        }
    }
}

fn production_reactivity_test_app(props: ProductionReactivityTestAppProps) -> Element {
    let mounted = use_signal(|| true);
    props.mounted.borrow_mut().get_or_insert(mounted);
    let profile = CompilationProfile::standard();
    let mut compiler = FormDefinition::compiler(props.scenario.data_schema.clone());
    if let Some(ui_schema) = &props.scenario.ui_schema {
        let bytes = serde_json::to_vec(ui_schema).expect("the workload UI schema should serialize");
        compiler = compiler.ui_schema(
            parse_ui_schema_v1(&bytes, &profile).expect("the workload UI schema should parse"),
        );
    }
    let definition = compiler
        .profile(profile)
        .compile()
        .expect("the workload definition should compile");
    let form = use_form(definition, props.scenario.initial_form_data.clone())
        .expect("the workload form should be created");
    let observer = Arc::new(TestRenderObserver {
        observations: props.observations.clone(),
    });
    let bound = RenderConfiguration::builder()
        .observer(observer)
        .build()
        .bind(&form)
        .expect("the workload should bind to production renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        if *mounted.read() {
            SchemaForm {
                form: bound,
                on_submit: move |_| {},
            }
        }
    }
}

fn exact_widget_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["name"],
        "properties": {
            "name": {
                "type": "string",
                "title": "Full name",
                "description": "Enter a full name",
                "minLength": 2
            }
        }
    }))
    .ui_schema(UiSchema::new(UiElement::Control(
        Control::new(Binding::root(JsonPointer::parse("/name").unwrap()))
            .widget(WidgetSymbol::parse("company:text").unwrap()),
    )))
    .compile()
    .expect("the exact widget definition should compile");
    let form = use_form(definition, json!({ "name": "Ada" }))
        .expect("the exact widget form should be created");
    let matcher_calls = Rc::new(RefCell::new(0));
    let renderer = Arc::new(ExactRenderer {
        matcher_calls: matcher_calls.clone(),
    });
    let controls = ControlRegistry::with_builtins()
        .widget(WidgetSymbol::parse("company:text").unwrap(), renderer)
        .matcher(
            10,
            Arc::new(CountingMatcher {
                calls: matcher_calls.clone(),
            }),
            Arc::new(InstrumentedRenderer {
                lifecycle: Rc::new(RefCell::new(HashMap::new())),
            }),
        )
        .matcher(
            10,
            Arc::new(CountingMatcher {
                calls: matcher_calls,
            }),
            Arc::new(InstrumentedRenderer {
                lifecycle: Rc::new(RefCell::new(HashMap::new())),
            }),
        );
    let bound = RenderConfiguration::builder()
        .controls(controls)
        .build()
        .bind(&form)
        .expect("an exact widget should bypass unrelated matchers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn extension_preflight_test_app(props: TestAppProps) -> Element {
    let namespace_a = ExtensionNamespace::parse("https://example.com/a").unwrap();
    let namespace_b = ExtensionNamespace::parse("https://example.com/b").unwrap();
    let optional_namespace = ExtensionNamespace::parse("https://example.com/optional").unwrap();
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "name": { "type": "string", "title": "Name" },
            "other": { "type": "string", "title": "Other" }
        }
    }))
    .ui_schema(
        UiSchema::new(UiElement::Stack(
            Stack::new([
                UiElement::Control(
                    Control::new(Binding::root(JsonPointer::parse("/name").unwrap()))
                        .widget(WidgetSymbol::parse("company:text").unwrap())
                        .meta(
                            ElementMeta::default()
                                .extension(namespace_b.clone(), json!({ "enabled": true }))
                                .extension(namespace_a.clone(), json!({ "enabled": true })),
                        ),
                ),
                UiElement::Auto(
                    Auto::new(Binding::root(JsonPointer::parse("").unwrap()))
                        .properties(PropertySelection::default().include("other"))
                        .meta(
                            ElementMeta::default()
                                .extension(optional_namespace, json!({ "preserved": true })),
                        ),
                ),
            ])
            .meta(
                ElementMeta::default()
                    .extension(namespace_b.clone(), json!({ "enabled": true }))
                    .extension(namespace_a.clone(), json!({ "enabled": true })),
            ),
        ))
        .require_extension(namespace_b.clone())
        .require_extension(namespace_a.clone()),
    )
    .compile()
    .unwrap();
    let form = use_form(definition, json!({ "name": "Ada", "other": "kept" })).unwrap();
    let before = form.reader().read().expect("form should be readable");

    let rejected_calls = Rc::new(RefCell::new(Vec::new()));
    let rejected = RenderConfiguration::builder()
        .controls(ControlRegistry::with_builtins().widget(
            WidgetSymbol::parse("company:text").unwrap(),
            Arc::new(ExactRenderer {
                matcher_calls: Rc::new(RefCell::new(0)),
            }),
        ))
        .extension(
            ExtensionNamespace::parse("https://example.com/a/").unwrap(),
            Arc::new(TestExtensionHandler {
                marker: "a/",
                preparations: rejected_calls.clone(),
                error: None,
            }),
        )
        .extension(
            namespace_b.clone(),
            Arc::new(TestExtensionHandler {
                marker: "b",
                preparations: rejected_calls.clone(),
                error: Some(ExtensionPrepareError::InvalidValue),
            }),
        )
        .build()
        .bind(&form)
        .err()
        .expect("missing and invalid required support must fail one atomic bind")
        .findings()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(rejected.len(), 3);
    assert!(matches!(
        &rejected[0],
        BindFinding::MissingRequiredExtension(missing) if missing == &namespace_a
    ));
    assert!(rejected[1..].iter().all(|finding| matches!(
        finding,
        BindFinding::InvalidRequiredExtension {
            namespace,
            error: ExtensionPrepareError::InvalidValue,
            ..
        } if namespace == &namespace_b
    )));
    assert_eq!(
        rejected_calls.borrow().as_slice(),
        ["https://example.com/b", "https://example.com/b"]
    );
    let after_rejected = form.reader().read().expect("form should be readable");
    assert_eq!(after_rejected.data_revision, before.data_revision);
    assert_eq!(after_rejected.state_revision, before.state_revision);
    assert_eq!(
        form.reader().form_data().expect("form should be readable"),
        json!({ "name": "Ada", "other": "kept" })
    );

    let preparations = Rc::new(RefCell::new(Vec::new()));
    let renderer = Arc::new(ExactRenderer {
        matcher_calls: Rc::new(RefCell::new(0)),
    });
    let bound = RenderConfiguration::builder()
        .controls(
            ControlRegistry::with_builtins()
                .widget(WidgetSymbol::parse("company:text").unwrap(), renderer),
        )
        .extension(
            namespace_b,
            Arc::new(TestExtensionHandler {
                marker: "b",
                preparations: preparations.clone(),
                error: None,
            }),
        )
        .extension(
            namespace_a,
            Arc::new(TestExtensionHandler {
                marker: "a",
                preparations: preparations.clone(),
                error: None,
            }),
        )
        .build()
        .bind(&form)
        .expect("required exact handlers should prepare atomically");
    assert_eq!(
        preparations.borrow().as_slice(),
        [
            "https://example.com/a",
            "https://example.com/a",
            "https://example.com/b",
            "https://example.com/b"
        ]
    );
    let after_bound = form.reader().read().expect("form should be readable");
    assert_eq!(after_bound.data_revision, before.data_revision);
    assert_eq!(after_bound.state_revision, before.state_revision);
    assert_eq!(
        form.reader().form_data().expect("form should be readable"),
        json!({ "name": "Ada", "other": "kept" })
    );
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |_| {},
        }
    }
}

fn renderer_bind_findings_test_app(props: TestAppProps) -> Element {
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "first": { "type": "string" },
            "second": { "type": "string" }
        }
    });
    let missing_definition = FormDefinition::compiler(data_schema.clone())
        .ui_schema(UiSchema::new(UiElement::Stack(Stack::new([
            UiElement::Control(
                Control::new(Binding::root(JsonPointer::parse("/first").unwrap()))
                    .widget(WidgetSymbol::parse("company:first").unwrap()),
            ),
            UiElement::Control(
                Control::new(Binding::root(JsonPointer::parse("/second").unwrap()))
                    .widget(WidgetSymbol::parse("company:second").unwrap()),
            ),
        ]))))
        .compile()
        .unwrap();
    let missing_form = use_form(
        missing_definition,
        json!({ "first": "Ada", "second": "Lovelace" }),
    )
    .unwrap();
    let missing = RenderConfiguration::default()
        .bind(&missing_form)
        .err()
        .expect("unregistered exact widgets must fail binding")
        .findings()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        missing,
        [
            BindFinding::MissingWidget(WidgetSymbol::parse("company:first").unwrap()),
            BindFinding::MissingWidget(WidgetSymbol::parse("company:second").unwrap()),
        ],
        "all missing exact widgets should be reported by one atomic preflight"
    );

    let item_extension = ExtensionNamespace::parse("https://example.com/item").unwrap();
    let array_widget = WidgetSymbol::parse("company:tags").unwrap();
    let item_widget = WidgetSymbol::parse("company:tag").unwrap();
    let missing_array_definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "tags": { "type": "array", "items": { "type": "string" } }
        }
    }))
    .ui_schema(
        UiSchema::new(UiElement::Control(
            Control::new(Binding::root(JsonPointer::parse("/tags").unwrap()))
                .widget(array_widget.clone())
                .item_template(UiElement::Control(
                    Control::new(Binding::item(JsonPointer::parse("").unwrap()))
                        .widget(item_widget.clone())
                        .meta(
                            ElementMeta::default()
                                .extension(item_extension.clone(), json!({ "enabled": true })),
                        ),
                )),
        ))
        .require_extension(item_extension.clone()),
    )
    .compile()
    .unwrap();
    let missing_array_form =
        use_form(missing_array_definition, json!({ "tags": ["first"] })).unwrap();
    let missing_array = RenderConfiguration::builder()
        .controls(ControlRegistry::with_builtins().widget(
            array_widget.clone(),
            Arc::new(ExactRenderer {
                matcher_calls: Rc::new(RefCell::new(0)),
            }),
        ))
        .build()
        .bind(&missing_array_form)
        .err()
        .expect("the registered array widget and missing item support must fail binding")
        .findings()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        missing_array,
        [
            BindFinding::MissingRequiredExtension(item_extension),
            BindFinding::UnsupportedCollectionWidget(array_widget),
            BindFinding::MissingWidget(item_widget),
        ],
        "array rejection and item-template preflight failures should be aggregated"
    );

    let generated_definition = FormDefinition::compiler(data_schema)
        .ui_schema(UiSchema::new(UiElement::Control(Control::new(
            Binding::root(JsonPointer::parse("/first").unwrap()),
        ))))
        .compile()
        .unwrap();
    let generated_form = use_form(
        generated_definition,
        json!({ "first": "Ada", "second": "Lovelace" }),
    )
    .unwrap();
    let renderer = || {
        Arc::new(InstrumentedRenderer {
            lifecycle: Rc::new(RefCell::new(HashMap::new())),
        }) as Arc<dyn ControlRenderer>
    };
    let tie = RenderConfiguration::builder()
        .controls(
            ControlRegistry::with_builtins()
                .matcher(
                    10,
                    Arc::new(CountingMatcher {
                        calls: Rc::new(RefCell::new(0)),
                    }),
                    renderer(),
                )
                .matcher(
                    10,
                    Arc::new(CountingMatcher {
                        calls: Rc::new(RefCell::new(0)),
                    }),
                    renderer(),
                ),
        )
        .build()
        .bind(&generated_form)
        .err()
        .expect("equal highest-priority matchers must fail binding")
        .findings()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(tie, [BindFinding::AmbiguousMatcher]);

    let floor_tie = RenderConfiguration::builder()
        .controls(ControlRegistry::with_builtins().matcher(
            schemaform_dioxus::render::BUILTIN_CONTROL_PRIORITY,
            Arc::new(CountingMatcher {
                calls: Rc::new(RefCell::new(0)),
            }),
            renderer(),
        ))
        .build()
        .bind(&generated_form)
        .err()
        .expect("a matcher tied with the built-in floor must be ambiguous")
        .findings()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(floor_tie, [BindFinding::AmbiguousMatcher]);

    let duplicate_symbol = WidgetSymbol::parse("company:first").unwrap();
    let duplicate = RenderConfiguration::builder()
        .controls(
            ControlRegistry::with_builtins()
                .widget(duplicate_symbol.clone(), renderer())
                .widget(duplicate_symbol.clone(), renderer())
                .widget(WidgetSymbol::parse("company:second").unwrap(), renderer()),
        )
        .build()
        .bind(&missing_form)
        .err()
        .expect("duplicate exact registrations must fail binding")
        .findings()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(duplicate, [BindFinding::AmbiguousWidget(duplicate_symbol)]);

    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| generated_form.clone());
    rsx! { form { input { "data-bind-findings-complete": "" } } }
}

fn matcher_priority_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "high": { "type": "string" },
            "low": { "type": "string" }
        }
    }))
    .unwrap();
    let form = use_form(definition, json!({ "high": "Ada", "low": "built in" })).unwrap();
    let matcher_calls = Rc::new(RefCell::new(0));
    let controls = ControlRegistry::with_builtins()
        .matcher(
            1,
            Arc::new(BindingMatcher {
                binding: "/high",
                calls: matcher_calls.clone(),
            }),
            Arc::new(PriorityRenderer {
                marker: "lower",
                matcher_calls: matcher_calls.clone(),
            }),
        )
        .matcher(
            10,
            Arc::new(BindingMatcher {
                binding: "/high",
                calls: matcher_calls.clone(),
            }),
            Arc::new(PriorityRenderer {
                marker: "highest",
                matcher_calls: matcher_calls.clone(),
            }),
        )
        .matcher(
            schemaform_dioxus::render::BUILTIN_CONTROL_PRIORITY - 1,
            Arc::new(BindingMatcher {
                binding: "/low",
                calls: matcher_calls.clone(),
            }),
            Arc::new(PriorityRenderer {
                marker: "below-floor",
                matcher_calls,
            }),
        );
    let bound = RenderConfiguration::builder()
        .controls(controls)
        .build()
        .bind(&form)
        .expect("the highest matcher and built-in floor should resolve deterministically");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn custom_array_item_widget_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "tags": {
                "type": "array",
                "items": { "type": "string", "default": "new" }
            }
        }
    }))
    .ui_schema(UiSchema::new(UiElement::Control(
        Control::new(Binding::root(JsonPointer::parse("/tags").unwrap())).item_template(
            UiElement::Control(
                Control::new(Binding::item(JsonPointer::parse("").unwrap()))
                    .widget(WidgetSymbol::parse("company:tag").unwrap()),
            ),
        ),
    )))
    .compile()
    .expect("the custom item widget definition should compile");
    let form = use_form(definition, json!({ "tags": ["first"] })).unwrap();
    let matcher_calls = Rc::new(RefCell::new(0));
    let bound = RenderConfiguration::builder()
        .controls(
            ControlRegistry::with_builtins()
                .widget(
                    WidgetSymbol::parse("company:tag").unwrap(),
                    Arc::new(ExactRenderer {
                        matcher_calls: matcher_calls.clone(),
                    }),
                )
                .matcher(
                    10,
                    Arc::new(CountingMatcher {
                        calls: matcher_calls.clone(),
                    }),
                    Arc::new(ExactRenderer { matcher_calls }),
                ),
        )
        .build()
        .bind(&form)
        .expect("the adapter-owned array should retain its custom item renderer");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn scalar_array_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["tags"],
        "properties": {
            "tags": {
                "type": "array",
                "title": "Tags",
                "minItems": 1,
                "maxItems": 3,
                "items": {
                    "type": "string",
                    "title": "Tag",
                    "default": "valid",
                    "minLength": 4
                }
            }
        }
    }))
    .expect("the scalar array data schema should compile");
    let form = use_form(definition, json!({ "tags": ["same", "same"] }))
        .expect("the browser array form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the scalar array should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn optional_array_presence_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "tags": {
                "type": "array",
                "title": "Tags",
                "default": ["seed"],
                "items": { "type": "string", "title": "Tag" }
            }
        }
    }))
    .expect("the optional array data schema should compile");
    let form = use_form(definition, json!({}))
        .expect("the missing optional array should remain constructible");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the optional array should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn fixed_object_array_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["people"],
        "properties": {
            "people": {
                "type": "array",
                "title": "People",
                "minItems": 1,
                "maxItems": 3,
                "items": {
                    "type": "object",
                    "title": "Person",
                    "description": "One person in the list.",
                    "additionalProperties": false,
                    "default": { "name": "New", "address": { "city": "Seed" } },
                    "required": ["name", "address"],
                    "properties": {
                        "name": { "type": "string", "title": "Name" },
                        "address": {
                            "type": "object",
                            "title": "Address",
                            "additionalProperties": false,
                            "required": ["city"],
                            "properties": {
                                "city": {
                                    "type": "string",
                                    "title": "City",
                                    "description": "City fallback help.",
                                    "minLength": 3
                                }
                            }
                        }
                    }
                }
            }
        }
    }))
    .expect("the fixed-object array data schema should compile");
    let duplicate = json!({ "name": "Ada", "address": { "city": "Rome" } });
    let form = use_form(
        definition,
        json!({ "people": [duplicate.clone(), duplicate] }),
    )
    .expect("the browser fixed-object array form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the fixed-object item template should bind to built-in renderers");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn authored_scalar_array_test_app(props: TestAppProps) -> Element {
    let pointer = |value| JsonPointer::parse(value).expect("the test binding should be valid");
    let ui_schema = UiSchema::new(UiElement::Control(
        Control::new(Binding::root(pointer("/tags")))
            .item_label(TextReference::localized("array.entry", "Entry fallback"))
            .item_template(UiElement::Control(Control::new(Binding::item(pointer(""))))),
    ));
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["tags"],
        "properties": {
            "tags": {
                "type": "array",
                "title": "Tags",
                "items": {
                    "type": "string",
                    "title": "Tag",
                    "minLength": 3
                }
            }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored scalar array item template should compile");
    let form = use_form(definition, json!({ "tags": ["same", "same"] }))
        .expect("the authored scalar array form should be created");
    let bound = RenderConfiguration::builder()
        .localizer(Arc::new(AuthoredTestLocalizer))
        .build()
        .bind(&form)
        .expect("the authored scalar array item template should bind");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn authored_array_test_app(props: TestAppProps) -> Element {
    let pointer = |value| JsonPointer::parse(value).expect("the test binding should be valid");
    let ui_schema = UiSchema::new(UiElement::Control(
        Control::new(Binding::root(pointer("/people"))).item_template(UiElement::Stack(
            Stack::new([
                UiElement::Text(Text::new(TextReference::localized(
                    "people.intro",
                    "Person details",
                ))),
                UiElement::Control(Control::new(Binding::item(pointer("/name")))),
                UiElement::Group(Group::new(
                    TextReference::localized("people.location", "Location"),
                    UiElement::Control(Control::new(Binding::item(pointer("/address/city")))),
                )),
            ]),
        )),
    ));
    let definition = FormDefinition::compiler(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["people"],
        "properties": {
            "people": {
                "type": "array",
                "title": "People",
                "items": {
                    "type": "object",
                    "required": ["name", "address"],
                    "properties": {
                        "name": { "type": "string", "title": "Name" },
                        "address": {
                            "type": "object",
                            "required": ["city"],
                            "properties": {
                                "city": {
                                    "type": "string",
                                    "title": "City",
                                    "minLength": 3
                                }
                            }
                        }
                    }
                }
            }
        }
    }))
    .ui_schema(ui_schema)
    .compile()
    .expect("the authored array item template should compile");
    let duplicate = json!({ "name": "Ada", "address": { "city": "Rome" } });
    let form = use_form(
        definition,
        json!({ "people": [duplicate.clone(), duplicate] }),
    )
    .expect("the authored array form should be created");
    let bound = RenderConfiguration::default()
        .bind(&form)
        .expect("the authored array item template should bind");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
        }
    }
}

fn constant_array_test_app(props: TestAppProps) -> Element {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["values"],
        "properties": {
            "values": {
                "type": "array",
                "title": "Values",
                "maxItems": 3,
                "items": { "const": "fixed", "title": "Value" }
            }
        }
    }))
    .unwrap();
    let form = use_form(definition, json!({ "values": ["fixed"] })).unwrap();
    let bound = RenderConfiguration::default().bind(&form).unwrap();
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |_| {},
        }
    }
}

#[wasm_bindgen_test]
async fn authored_ui_schema_renders_semantics_and_preserves_form_behavior() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(authored_ui_test_app).await;
    let stack = root
        .query_selector("[data-schemaform-stack]")
        .expect("the stack selector should be valid")
        .expect("the authored stack should render");
    let text = stack
        .query_selector("[data-schemaform-text]")
        .expect("the text selector should be valid")
        .expect("the authored text should render");
    assert_eq!(
        text.text_content().as_deref(),
        Some("Use <strong>localized plain text</strong>.")
    );
    assert!(
        text.query_selector("strong")
            .expect("the strong selector should be valid")
            .is_none(),
        "static UI-schema text must be escaped plain text"
    );

    let second = input_with_binding(&root, "/second");
    let second_container = second
        .parent_element()
        .expect("the second input should have a control container");
    assert!(
        text.next_element_sibling()
            .is_some_and(|sibling| sibling.is_same_node(Some(&second_container))),
        "authored control order must be preserved"
    );
    let group = second_container
        .next_element_sibling()
        .expect("the authored group should follow the second control");
    assert_eq!(
        group.get_attribute("data-schemaform-group").as_deref(),
        Some("")
    );
    assert_eq!(
        group
            .query_selector("legend")
            .expect("the legend selector should be valid")
            .and_then(|legend| legend.text_content())
            .as_deref(),
        Some("Localized primary details")
    );
    assert!(maybe_input_with_binding(&root, "/hidden").is_none());

    let label = root
        .query_selector(&format!("label[for='{}']", second.id()))
        .expect("the label selector should be valid")
        .expect("the authored control should remain labeled");
    assert_eq!(
        label.text_content().as_deref(),
        Some("Localized second field")
    );
    let described_by = second
        .get_attribute("aria-describedby")
        .expect("authored help should describe its control");
    let help = root
        .query_selector(&format!("#{described_by}"))
        .expect("the help selector should be valid")
        .expect("the authored help should render");
    assert_eq!(
        help.text_content().as_deref(),
        Some("Localized family-name help.")
    );
    accessibility_checkpoint(
        "authored-ui",
        "authored_ui_schema_renders_semantics_and_preserves_form_behavior",
        &root,
    )
    .await;

    dispatch_input(&second, "");
    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    next_microtask().await;
    assert!(submitted.borrow().is_none());
    poll_dom(|| {
        root.query_selector("[data-validation-finding='minLength']")
            .ok()
            .flatten()
    })
    .await;

    dispatch_input(&second, "Byron");
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        snapshot.form_data(),
        &json!({ "first": "Ada", "second": "Byron", "hidden": "preserved" })
    );
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({ "first": "Ada", "second": "Byron", "hidden": "preserved" })
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn tabs_support_keyboard_navigation_adapter_local_selection_and_summary_focus() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(tabs_test_app).await;
    let tablist = root
        .query_selector("[role='tablist']")
        .unwrap()
        .expect("the tabs should render a tablist");
    assert_eq!(tablist.get_attribute("aria-label").as_deref(), Some("Tabs"));
    let tabs = tablist.query_selector_all("[role='tab']").unwrap();
    let panels = root.query_selector_all("[role='tabpanel']").unwrap();
    assert_eq!(tabs.length(), 2);
    assert_eq!(panels.length(), 2);
    let first_tab: web_sys::HtmlElement = tabs.get(0).unwrap().dyn_into().unwrap();
    let second_tab: web_sys::HtmlElement = tabs.get(1).unwrap().dyn_into().unwrap();
    let first_panel: web_sys::HtmlElement = panels.get(0).unwrap().dyn_into().unwrap();
    let second_panel: web_sys::HtmlElement = panels.get(1).unwrap().dyn_into().unwrap();
    assert_eq!(first_tab.text_content().as_deref(), Some("Account"));
    assert_eq!(second_tab.text_content().as_deref(), Some("Contact"));
    assert_eq!(
        first_tab.get_attribute("aria-selected").as_deref(),
        Some("true")
    );
    assert_eq!(first_tab.tab_index(), 0);
    assert_eq!(second_tab.tab_index(), -1);
    assert!(!first_panel.hidden());
    assert!(second_panel.hidden());
    assert_eq!(
        first_tab.get_attribute("aria-controls").as_deref(),
        Some(first_panel.id().as_str())
    );
    assert_eq!(
        first_panel.get_attribute("aria-labelledby").as_deref(),
        Some(first_tab.id().as_str())
    );

    let initial = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    first_tab.focus().unwrap();
    dispatch_keydown(&first_tab, "ArrowRight");
    wait_for_tab_selection(&second_tab, &second_panel, "ArrowRight", true).await;
    dispatch_keydown(&second_tab, "ArrowRight");
    wait_for_tab_selection(&first_tab, &first_panel, "ArrowRight wrap", true).await;
    dispatch_keydown(&first_tab, "ArrowLeft");
    wait_for_tab_selection(&second_tab, &second_panel, "ArrowLeft wrap", true).await;
    dispatch_keydown(&second_tab, "Home");
    wait_for_tab_selection(&first_tab, &first_panel, "Home", true).await;
    dispatch_keydown(&first_tab, "End");
    wait_for_tab_selection(&second_tab, &second_panel, "End", true).await;

    first_tab.focus().unwrap();
    wait_for_tab_selection(&first_tab, &first_panel, "automatic focus activation", true).await;
    assert!(!dispatch_keydown(&first_tab, "Enter"));
    wait_for_tab_selection(&first_tab, &first_panel, "Enter", true).await;
    second_tab.focus().unwrap();
    wait_for_tab_selection(
        &second_tab,
        &second_panel,
        "automatic focus activation",
        true,
    )
    .await;
    assert!(!dispatch_keydown(&second_tab, " "));
    wait_for_tab_selection(&second_tab, &second_panel, "Space", true).await;
    second_tab.click();
    wait_for_tab_selection(&second_tab, &second_panel, "click", true).await;
    assert!(
        dispatch_keydown(&second_tab, "Tab"),
        "normal Tab navigation must not be canceled by the tablist"
    );
    let after_keyboard = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(after_keyboard.data_revision, initial.data_revision);
    assert_eq!(after_keyboard.state_revision, initial.state_revision);
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({ "name": "Ada", "email": "Li" })
    );

    first_tab.click();
    wait_for_tab_selection(&first_tab, &first_panel, "first click", false).await;
    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    dispatch_submit(&form);
    let summary_action = poll_dom(|| {
        root.query_selector("[data-finding-summary] [data-finding='minLength'] button")
            .ok()
            .flatten()?
            .dyn_into::<web_sys::HtmlElement>()
            .ok()
    })
    .await;
    assert!(second_panel.hidden());
    let after_submit = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    summary_action.click();
    let email = input_with_binding(&root, "/email");
    wait_for_tab_selection(&second_tab, &second_panel, "summary", false).await;
    wait_for_input_focus(&email, "tab summary").await;
    let after_summary = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(after_summary.data_revision, after_submit.data_revision);
    assert_eq!(after_summary.state_revision, after_submit.state_revision);
    assert!(submitted.borrow().is_none());
    accessibility_checkpoint(
        "tabs-blocked",
        "tabs_support_keyboard_navigation_adapter_local_selection_and_summary_focus",
        &root,
    )
    .await;

    root.remove();
}

#[wasm_bindgen_test]
async fn responsive_grid_preserves_dom_focus_order_and_behavior_at_320_css_pixels() {
    let MountedViewportTestApp {
        iframe,
        root,
        form_handle,
        submitted,
        wide_grid_columns,
    } = mount_test_app_in_viewport(responsive_grid_test_app, 800).await;
    let grid = root
        .query_selector("[data-schemaform-grid]")
        .unwrap()
        .expect("the semantic grid should render");
    let cells = grid
        .query_selector_all("[data-schemaform-grid-cell]")
        .unwrap();
    assert_eq!(cells.length(), 2);
    let first_cell: web_sys::Element = cells.get(0).unwrap().unchecked_into();
    let second_cell: web_sys::Element = cells.get(1).unwrap().unchecked_into();
    assert_eq!(
        first_cell.get_attribute("data-compact-span").as_deref(),
        Some("12")
    );
    assert_eq!(
        first_cell.get_attribute("data-wide-span").as_deref(),
        Some("4")
    );
    assert_eq!(
        second_cell.get_attribute("data-compact-span").as_deref(),
        Some("12")
    );
    assert_eq!(
        second_cell.get_attribute("data-wide-span").as_deref(),
        Some("8")
    );
    assert_eq!(
        wide_grid_columns,
        ("span 4".to_owned(), "span 8".to_owned())
    );
    let grid_styles = iframe
        .content_document()
        .unwrap()
        .query_selector("style")
        .unwrap()
        .unwrap()
        .text_content()
        .unwrap();
    assert!(grid_styles.contains("@media (min-width:640px)"));

    let first = input_with_binding(&root, "/first");
    let second = input_with_binding(&root, "/second");
    let wide_focus_order = focusable_order(&root);
    assert!(first_cell.contains(Some(&first)));
    assert!(second_cell.contains(Some(&second)));
    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .unwrap()
        .unchecked_into();
    let parent_body = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .body()
        .unwrap();
    parent_body.append_child(&root).unwrap();
    dispatch_submit(&form);
    let wide_snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        wide_snapshot.form_data(),
        &json!({ "first": "Ada", "second": "Byron" })
    );
    *submitted.borrow_mut() = None;
    iframe
        .content_document()
        .unwrap()
        .body()
        .unwrap()
        .append_child(&root)
        .unwrap();
    second.focus().unwrap();

    iframe.set_width("320");
    poll_dom(|| {
        (iframe.content_window()?.inner_width().ok()?.as_f64() == Some(320.0)).then_some(())
    })
    .await;
    assert_eq!(
        computed_grid_column_end(&iframe.content_window().unwrap(), &first_cell),
        "span 12"
    );
    assert_eq!(
        computed_grid_column_end(&iframe.content_window().unwrap(), &second_cell),
        "span 12"
    );

    let compact_grid = root
        .query_selector("[data-schemaform-grid]")
        .unwrap()
        .unwrap();
    let compact_cells = compact_grid
        .query_selector_all("[data-schemaform-grid-cell]")
        .unwrap();
    let compact_first_cell: web_sys::Element = compact_cells.get(0).unwrap().unchecked_into();
    let compact_second_cell: web_sys::Element = compact_cells.get(1).unwrap().unchecked_into();
    assert!(grid.is_same_node(Some(&compact_grid)));
    assert!(first_cell.is_same_node(Some(&compact_first_cell)));
    assert!(second_cell.is_same_node(Some(&compact_second_cell)));
    poll_dom(|| {
        let current_first = maybe_input_with_binding(&root, "/first")?;
        let current_second = maybe_input_with_binding(&root, "/second")?;
        (first.is_same_node(Some(&current_first)) && second.is_same_node(Some(&current_second)))
            .then_some(())
    })
    .await;
    assert_eq!(focusable_order(&root), wide_focus_order);
    let focused = iframe
        .content_document()
        .unwrap()
        .active_element()
        .expect("the focused control should remain focused");
    assert_eq!(focused.id(), second.id());

    // Dioxus delegates events through the document where the VDOM was mounted.
    parent_body.append_child(&root).unwrap();
    dispatch_input(&first, "Grace");
    dispatch_input(&second, "Li");
    dispatch_submit(&form);
    next_microtask().await;
    assert!(submitted.borrow().is_none());
    dispatch_input(&second, "Hopper");
    dispatch_submit(&form);
    let compact_snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        compact_snapshot.form_data(),
        &json!({ "first": "Grace", "second": "Hopper" })
    );
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        compact_snapshot.form_data().clone()
    );

    root.remove();
    iframe.remove();
}

#[wasm_bindgen_test]
async fn responsive_grid_behavior_matches_the_active_css_viewport() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(responsive_grid_test_app).await;
    let cells = root
        .query_selector_all("[data-schemaform-grid-cell]")
        .unwrap();
    let first_cell: web_sys::Element = cells.get(0).unwrap().unchecked_into();
    let second_cell: web_sys::Element = cells.get(1).unwrap().unchecked_into();
    let window = web_sys::window().unwrap();
    let compact = window.inner_width().unwrap().as_f64().unwrap() < 640.0;
    assert_eq!(
        computed_grid_column_end(&window, &first_cell),
        if compact { "span 12" } else { "span 4" }
    );
    assert_eq!(
        computed_grid_column_end(&window, &second_cell),
        if compact { "span 12" } else { "span 8" }
    );

    let first = input_with_binding(&root, "/first");
    let second = input_with_binding(&root, "/second");
    assert_eq!(first.tab_index(), 0);
    assert_eq!(second.tab_index(), 0);
    assert!(
        focusable_order(&root)
            .windows(2)
            .any(|pair| pair == ["/first", "/second"])
    );
    second.focus().unwrap();
    assert_focused(&second);

    dispatch_input(&first, "Grace");
    dispatch_input(&second, "Li");
    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .unwrap()
        .unchecked_into();
    dispatch_submit(&form);
    next_microtask().await;
    assert!(submitted.borrow().is_none());
    dispatch_input(&second, "Hopper");
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        snapshot.form_data(),
        &json!({ "first": "Grace", "second": "Hopper" })
    );
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        snapshot.form_data().clone()
    );
    assert!(first.is_same_node(Some(&input_with_binding(&root, "/first"))));
    assert!(second.is_same_node(Some(&input_with_binding(&root, "/second"))));
    accessibility_checkpoint(
        "grid",
        "responsive_grid_behavior_matches_the_active_css_viewport",
        &root,
    )
    .await;

    root.remove();
}

#[wasm_bindgen_test]
async fn explicit_auto_renders_only_at_its_authored_position() {
    let MountedTestApp {
        root, submitted, ..
    } = mount_test_app(auto_region_test_app).await;
    let stack = root
        .query_selector("[data-schemaform-stack]")
        .unwrap()
        .expect("the authored stack should render");
    let texts = stack.query_selector_all("[data-schemaform-text]").unwrap();
    assert_eq!(texts.length(), 2);
    let opening_text: web_sys::Element = texts.get(0).unwrap().dyn_into().unwrap();
    let closing_text: web_sys::Element = texts.get(1).unwrap().dyn_into().unwrap();
    assert_eq!(
        opening_text.text_content().as_deref(),
        Some("Generated fields")
    );
    assert_eq!(closing_text.text_content().as_deref(), Some("End fields"));

    let second = input_with_binding(&root, "/second");
    let first = input_with_binding(&root, "/first");
    let second_container = second.parent_element().unwrap();
    let first_container = first.parent_element().unwrap();
    assert!(
        opening_text
            .next_element_sibling()
            .is_some_and(|sibling| sibling.is_same_node(Some(&second_container)))
    );
    assert!(
        second_container
            .next_element_sibling()
            .is_some_and(|sibling| sibling.is_same_node(Some(&first_container)))
    );
    assert!(
        first_container
            .next_element_sibling()
            .is_some_and(|sibling| sibling.is_same_node(Some(&closing_text)))
    );
    assert!(maybe_input_with_binding(&root, "/hidden").is_none());
    accessibility_checkpoint(
        "auto",
        "explicit_auto_renders_only_at_its_authored_position",
        &root,
    )
    .await;

    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        snapshot.form_data(),
        &json!({ "first": "Ada", "second": "Lovelace", "hidden": "preserved" })
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn generated_string_control_mounts_edits_and_submits_an_immutable_snapshot() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(string_test_app).await;
    let input: HtmlInputElement = poll_dom(|| {
        root.query_selector("form input[name='/name']")
            .expect("the input selector should be valid")
            .map(|element| element.dyn_into().expect("the control should be an input"))
    })
    .await;
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({ "name": "Ada" })
    );
    assert_eq!(input.value(), "Ada");

    let inputs = root
        .query_selector_all("input[name='/name']")
        .expect("the input selector should be valid");
    assert_eq!(inputs.length(), 2);
    let second_input: HtmlInputElement = inputs
        .get(1)
        .expect("the second form should render an input")
        .dyn_into()
        .expect("the second control should be an input");
    assert_ne!(input.id(), second_input.id());

    let input_id = input.id();
    let label = root
        .query_selector(&format!("label[for='{input_id}']"))
        .expect("the label selector should be valid")
        .expect("the generated control should have a programmatic label");
    assert_eq!(label.text_content().as_deref(), Some("Full name"));
    accessibility_checkpoint(
        "string",
        "generated_string_control_mounts_edits_and_submits_an_immutable_snapshot",
        &root,
    )
    .await;

    dispatch_input(&input, "Grace");
    let grace_data = json!({ "name": "Grace" });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == grace_data)
            .then_some(())
    })
    .await;

    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    assert!(form.no_validate());
    dispatch_submit(&form);

    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &grace_data);

    dispatch_input(&input, "Lin");
    let lin_data = json!({ "name": "Lin" });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == lin_data)
            .then_some(())
    })
    .await;
    assert_eq!(snapshot.form_data(), &grace_data);

    root.remove();
}

#[wasm_bindgen_test]
async fn handle_operations_report_reentrant_transaction_borrow_conflicts() {
    let (
        MountedTestApp {
            root, form_handle, ..
        },
        errors,
    ) = mount_test_app_with_errors(string_test_app).await;
    let before = form_handle
        .reader()
        .form_data()
        .expect("form should be readable");
    let root_reader = form_handle
        .node(
            form_handle
                .reader()
                .read()
                .expect("form should be readable")
                .root,
        )
        .expect("the form should not be mutably borrowed before the transaction")
        .expect("the root node should be readable before the transaction");
    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    let captured = form_handle.clone();
    let nested_called = Rc::new(RefCell::new(false));
    let nested_called_in_transaction = nested_called.clone();
    let errors_in_transaction = errors.clone();

    let transition = form_handle
        .try_transact(move |_| {
            assert!(matches!(
                captured.reader().read(),
                Err(HandleError::BorrowConflict)
            ));
            assert!(matches!(
                captured.reader().form_data(),
                Err(HandleError::BorrowConflict)
            ));
            assert!(matches!(
                captured.node(root_reader.identity()),
                Err(HandleError::BorrowConflict)
            ));
            assert!(matches!(
                root_reader.read(),
                Err(HandleError::BorrowConflict)
            ));
            assert!(matches!(
                captured.prepare_submission(),
                Err(HandleError::BorrowConflict)
            ));
            assert!(matches!(captured.reset(), Err(HandleError::BorrowConflict)));
            dispatch_submit(&form);
            assert_eq!(
                errors_in_transaction.borrow().as_slice(),
                &[HandleError::BorrowConflict]
            );
            let nested_called = nested_called_in_transaction.clone();
            let nested = captured.try_transact(move |_| {
                *nested_called.borrow_mut() = true;
                Ok::<_, ()>(())
            });
            assert!(matches!(
                nested,
                Err(HandleTransactionError::Handle(HandleError::BorrowConflict))
            ));
            Ok::<_, ()>(())
        })
        .expect("the outer transaction should retain its borrow and complete");

    assert!(transition.is_empty());
    assert!(!*nested_called.borrow());
    assert_ne!(
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element()
            .map(|element| element.id()),
        root.query_selector("[data-finding-summary]")
            .unwrap()
            .map(|element| element.id())
    );
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        before
    );

    let closure_failure = form_handle.try_transact(|_| Err("closure failed"));
    assert!(matches!(
        closure_failure,
        Err(HandleTransactionError::Transaction(
            schemaform::form::TransactionError::Closure("closure failed")
        ))
    ));
    let commit_failure = form_handle.try_transact(|draft| {
        draft.remove(&JsonPointer::parse("/missing").unwrap());
        Ok::<_, ()>(())
    });
    assert!(matches!(
        commit_failure,
        Err(HandleTransactionError::Transaction(
            schemaform::form::TransactionError::Commit(
                schemaform::form::HostCommitError::InvalidOperation
            )
        ))
    ));

    root.remove();
}

#[wasm_bindgen_test]
async fn every_in_profile_business_schema_executes_through_the_default_browser_adapter() {
    let expected = product_cases::fixtures()
        .into_iter()
        .filter(product_cases::BusinessSchemaFixture::is_in_profile)
        .map(|fixture| fixture.id)
        .collect::<HashSet<_>>();
    let MountedBusinessCorpus {
        root,
        handles,
        submitted,
    } = mount_business_corpus_test_app().await;

    assert_eq!(expected.len(), 15);
    assert_eq!(
        handles.borrow().keys().cloned().collect::<HashSet<_>>(),
        expected
    );
    for id in &expected {
        let fixture = product_cases::fixtures()
            .into_iter()
            .find(|fixture| fixture.id == *id)
            .expect("the rendered business fixture should remain embedded");
        let selector = format!("[data-business-schema-fixture='{id}']");
        let fixture_root = root
            .query_selector(&selector)
            .expect("the business fixture selector should be valid")
            .unwrap_or_else(|| panic!("business fixture {id} should render"));
        let direct_controls = fixture
            .expected_controls
            .iter()
            .filter(|control| !control.binding[1..].contains('/'))
            .collect::<Vec<_>>();
        assert!(
            !direct_controls.is_empty(),
            "business fixture {id} should declare a direct execution control"
        );
        let direct_arrays = direct_controls
            .iter()
            .filter(|control| control.kind == "homogeneous-array")
            .count();
        assert!(
            usize::try_from(
                fixture_root
                    .query_selector_all("fieldset[data-schemaform-array]")
                    .expect("the array selector should be valid")
                    .length()
            )
            .expect("the browser array count should fit usize")
                >= direct_arrays,
            "business fixture {id} should render every direct declared array"
        );
        for control in direct_controls
            .iter()
            .filter(|control| control.kind != "homogeneous-array")
        {
            let element = fixture_root
                .query_selector(&format!("[name='{}']", control.binding))
                .expect("the expected control selector should be valid");
            assert!(
                element.is_some(),
                "business fixture {id} should render declared control {}",
                control.binding
            );
        }
        let form: HtmlFormElement = fixture_root
            .query_selector("form")
            .expect("the form selector should be valid")
            .expect("the business fixture should render a form")
            .dyn_into()
            .expect("the rendered form should be an HTML form");
        let handle = handles
            .borrow()
            .get(id)
            .cloned()
            .expect("the mounted business fixture should expose its handle");
        let before = handle
            .reader()
            .form_data()
            .expect("form should be readable");
        let execution_control = direct_controls
            .iter()
            .find(|control| {
                matches!(
                    control.kind.as_str(),
                    "string" | "nullable-string" | "sensitive-string"
                )
            })
            .or_else(|| {
                direct_controls
                    .iter()
                    .find(|control| control.kind == "boolean")
            })
            .or_else(|| {
                direct_controls
                    .iter()
                    .find(|control| control.kind == "choice")
            })
            .or_else(|| {
                direct_controls
                    .iter()
                    .find(|control| control.kind == "homogeneous-array")
            })
            .expect("each business fixture should expose an editable execution control");
        match execution_control.kind.as_str() {
            "string" | "nullable-string" | "sensitive-string" => {
                let input = input_with_binding(&fixture_root, &execution_control.binding);
                dispatch_input(&input, "qualification");
            }
            "boolean" => {
                let input = input_with_binding(&fixture_root, &execution_control.binding);
                dispatch_checkbox_input(&input, true);
            }
            "choice" => {
                let select = select_with_binding(&fixture_root, &execution_control.binding);
                dispatch_select_alternative(&select);
            }
            "homogeneous-array" => {
                let add: web_sys::HtmlElement = fixture_root
                    .query_selector("fieldset[data-schemaform-array] button[data-append-item]")
                    .expect("the array add selector should be valid")
                    .unwrap_or_else(|| {
                        panic!("business fixture {id} should expose its built-in array add action")
                    })
                    .dyn_into()
                    .expect("the array add action should be a button");
                add.click();
            }
            kind => panic!("business fixture {id} has unsupported execution kind {kind}"),
        }
        poll_dom(|| {
            (handle
                .reader()
                .form_data()
                .expect("form should be readable")
                != before)
                .then_some(())
        })
        .await;

        dispatch_submit(&form);
        next_microtask().await;
        assert!(
            handle
                .reader()
                .read()
                .expect("form should be readable")
                .submission_attempted
                || submitted.borrow().contains(id),
            "business fixture {id} should execute browser submission"
        );
    }

    root.remove();
}

#[wasm_bindgen_test]
async fn array_presence_repairs_missing_incompatible_and_optional_data_in_the_browser() {
    let MountedTestApp {
        root, form_handle, ..
    } = mount_test_app(optional_array_presence_test_app).await;
    let array = poll_dom(|| {
        root.query_selector("fieldset[data-schemaform-array]")
            .expect("the array selector should be valid")
    })
    .await;

    let materialize = array
        .query_selector("button[data-materialize]")
        .unwrap()
        .expect("a missing array should expose materialization")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    materialize.click();
    poll_dom(|| {
        (form_handle.reader().form_data().unwrap() == json!({ "tags": ["seed"] })
            && array
                .query_selector_all("[data-array-item]")
                .unwrap()
                .length()
                == 1)
            .then_some(())
    })
    .await;
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        let status = array.query_selector("[data-array-status]").ok()??;
        (focused.id() == array.id() && status.text_content().as_deref() == Some("Tags added."))
            .then_some(())
    })
    .await;

    let remove = array
        .query_selector("button[data-remove-value]")
        .unwrap()
        .expect("a present optional array should expose removal")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    remove.click();
    poll_dom(|| (form_handle.reader().form_data().unwrap() == json!({})).then_some(())).await;
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        let status = array.query_selector("[data-array-status]").ok()??;
        (focused.id() == array.id() && status.text_content().as_deref() == Some("Tags removed."))
            .then_some(())
    })
    .await;

    form_handle
        .try_transact(|draft| {
            draft.set(&JsonPointer::parse("/tags").unwrap(), json!("legacy"));
            Ok::<_, ()>(())
        })
        .expect("the host should install incompatible array data");
    let incompatible = poll_dom(|| {
        array
            .query_selector("[data-incompatible-value]")
            .expect("the incompatible-value selector should be valid")
    })
    .await;
    assert_eq!(incompatible.text_content().as_deref(), Some("\"legacy\""));

    let replace = array
        .query_selector("button[data-replace-value]")
        .unwrap()
        .expect("an incompatible array should expose replacement")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    replace.click();
    poll_dom(|| {
        (form_handle.reader().form_data().unwrap() == json!({ "tags": ["seed"] })).then_some(())
    })
    .await;
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        let status = array.query_selector("[data-array-status]").ok()??;
        (focused.id() == array.id() && status.text_content().as_deref() == Some("Tags replaced."))
            .then_some(())
    })
    .await;

    root.remove();
}

#[wasm_bindgen_test]
async fn scalar_array_append_and_remove_keep_surviving_dom_identity_and_submit() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(scalar_array_test_app).await;
    let array = poll_dom(|| {
        root.query_selector("fieldset[data-schemaform-array]")
            .expect("the array selector should be valid")
    })
    .await;
    let initial = array
        .query_selector_all("input")
        .expect("the item selector should be valid");
    assert_eq!(initial.length(), 2);
    let first: HtmlInputElement = initial.get(0).unwrap().dyn_into().unwrap();
    let second: HtmlInputElement = initial.get(1).unwrap().dyn_into().unwrap();
    assert_eq!(first.name(), "/tags/0");
    assert_eq!(second.name(), "/tags/1");
    assert_ne!(first.id(), second.id());
    let second_id = second.id();
    let initial_projection = form_handle
        .reader()
        .read()
        .expect("form should be readable");

    array
        .query_selector("button[data-append-item]")
        .unwrap()
        .expect("append should be available below maxItems")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    let appended_data = json!({ "tags": ["same", "same", "valid"] });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == appended_data)
            .then_some(())
    })
    .await;
    let appended_projection = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_ne!(
        appended_projection.data_revision,
        initial_projection.data_revision
    );
    assert_ne!(
        appended_projection.state_revision,
        initial_projection.state_revision
    );
    let appended = poll_dom(|| {
        let inputs = array.query_selector_all("input").ok()?;
        (inputs.length() == 3).then_some(inputs)
    })
    .await;
    assert_eq!(
        appended
            .get(1)
            .unwrap()
            .dyn_into::<HtmlInputElement>()
            .unwrap()
            .id(),
        second_id
    );
    assert!(
        array
            .query_selector("button[data-append-item]")
            .unwrap()
            .is_none()
    );

    let second: HtmlInputElement = appended.get(1).unwrap().dyn_into().unwrap();
    dispatch_input(&second, "x");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "tags": ["same", "x", "valid"] }))
        .then_some(())
    })
    .await;
    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    dispatch_submit(&form);
    poll_dom(|| {
        array
            .query_selector("[data-validation-finding='minLength']")
            .ok()
            .flatten()
    })
    .await;
    assert!(submitted.borrow().is_none());

    let before_remove = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    array
        .query_selector("[data-array-item] button[data-remove-item]")
        .unwrap()
        .expect("an item should expose identity-targeted removal")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    let removed_data = json!({ "tags": ["x", "valid"] });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == removed_data)
            .then_some(())
    })
    .await;
    let after_remove = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_ne!(after_remove.data_revision, before_remove.data_revision);
    assert_ne!(after_remove.state_revision, before_remove.state_revision);
    let shifted = poll_dom(|| {
        let inputs = array.query_selector_all("input").ok()?;
        (inputs.length() == 2).then_some(inputs)
    })
    .await;
    let survivor: HtmlInputElement = shifted.get(0).unwrap().dyn_into().unwrap();
    assert_eq!(survivor.id(), second_id);
    assert_eq!(survivor.name(), "/tags/0");
    assert!(
        survivor
            .parent_element()
            .unwrap()
            .query_selector("[data-validation-finding='minLength']")
            .unwrap()
            .is_some()
    );

    dispatch_input(&survivor, "valid");
    let repaired_data = json!({ "tags": ["valid", "valid"] });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == repaired_data)
            .then_some(())
    })
    .await;
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &repaired_data);

    root.remove();
}

#[wasm_bindgen_test]
async fn scalar_array_structural_actions_preserve_dom_identity_focus_and_announcements() {
    let MountedTestApp {
        root, form_handle, ..
    } = mount_test_app(scalar_array_test_app).await;
    accessibility_checkpoint(
        "array-scalar",
        "scalar_array_structural_actions_preserve_dom_identity_focus_and_announcements",
        &root,
    )
    .await;
    let array = poll_dom(|| {
        root.query_selector("fieldset[data-schemaform-array]")
            .expect("the array selector should be valid")
    })
    .await;
    let initial = array.query_selector_all("input").unwrap();
    let first: HtmlInputElement = initial.get(0).unwrap().dyn_into().unwrap();
    let second: HtmlInputElement = initial.get(1).unwrap().dyn_into().unwrap();
    let first_id = first.id();
    let second_id = second.id();
    let second_identity = control_with_binding(&form_handle, "/tags/1");
    second.focus().unwrap();
    second.blur().unwrap();
    poll_dom(|| {
        form_handle
            .node(second_identity)
            .ok()??
            .read()
            .ok()??
            .touched
            .then_some(())
    })
    .await;
    let status = array
        .query_selector("[data-array-status]")
        .unwrap()
        .expect("the array should expose a structural status message");
    assert_eq!(status.get_attribute("role").as_deref(), Some("status"));
    assert_eq!(status.get_attribute("aria-live").as_deref(), Some("polite"));
    assert_eq!(status.get_attribute("aria-atomic").as_deref(), Some("true"));
    assert_eq!(
        array
            .query_selector("button[data-append-item]")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Add Tags item")
    );

    let rows = array.query_selector_all("[data-array-item]").unwrap();
    let insert = rows
        .get(1)
        .unwrap()
        .dyn_into::<web_sys::Element>()
        .unwrap()
        .query_selector("button[data-insert-item-before]")
        .unwrap()
        .expect("insert-before should be available below maxItems")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    assert_eq!(
        insert.text_content().as_deref(),
        Some("Insert Tags item before")
    );
    assert_eq!(
        insert.get_attribute("aria-label").as_deref(),
        Some("Insert Tags item before position 2")
    );
    insert.focus().unwrap();
    insert.click();
    let inserted_data = json!({ "tags": ["same", "valid", "same"] });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == inserted_data)
            .then_some(())
    })
    .await;
    let inserted = poll_dom(|| {
        let inputs = array.query_selector_all("input").ok()?;
        (inputs.length() == 3).then_some(inputs)
    })
    .await;
    let inserted_input: HtmlInputElement = inserted.get(1).unwrap().dyn_into().unwrap();
    let inserted_id = inserted_input.id();
    assert_focused(&inserted_input);
    assert_eq!(
        status.text_content().as_deref(),
        Some("Tags item inserted at position 2.")
    );
    let current_first: HtmlInputElement = inserted.get(0).unwrap().dyn_into().unwrap();
    let current_second: HtmlInputElement = inserted.get(2).unwrap().dyn_into().unwrap();
    assert!(first.is_same_node(Some(&current_first)));
    assert!(second.is_same_node(Some(&current_second)));

    let move_down = array
        .query_selector_all("[data-array-item]")
        .unwrap()
        .get(0)
        .unwrap()
        .dyn_into::<web_sys::Element>()
        .unwrap()
        .query_selector("button[data-move-item-down]")
        .unwrap()
        .expect("the first row should move down")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    let move_down_id = move_down.id();
    assert_eq!(
        move_down.get_attribute("aria-label").as_deref(),
        Some("Move Tags item at position 1 down")
    );
    move_down.focus().unwrap();
    move_down.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "tags": ["valid", "same", "same"] }))
        .then_some(())
    })
    .await;
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        (focused.id() == move_down_id).then_some(())
    })
    .await;
    poll_dom(|| {
        array
            .query_selector("[data-array-status]")
            .ok()
            .flatten()?
            .text_content()?
            .contains("moved down")
            .then_some(())
    })
    .await;
    assert_eq!(
        status.text_content().as_deref(),
        Some("Tags item moved down to position 2.")
    );
    assert_eq!(input_with_binding(&root, "/tags/1").id(), first_id);
    assert_eq!(input_with_binding(&root, "/tags/2").id(), second_id);

    let move_up = array
        .query_selector(&format!("button[id='{first_id}-move-up']"))
        .unwrap()
        .expect("the moved row should move back up")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    move_up.focus().unwrap();
    move_up.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "tags": ["same", "valid", "same"] }))
        .then_some(())
    })
    .await;
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        (focused.id() == format!("{first_id}-move-down")).then_some(())
    })
    .await;
    poll_dom(|| {
        array
            .query_selector("[data-array-status]")
            .ok()
            .flatten()?
            .text_content()?
            .contains("moved up")
            .then_some(())
    })
    .await;
    assert_eq!(
        status.text_content().as_deref(),
        Some("Tags item moved up to position 1.")
    );
    assert_eq!(input_with_binding(&root, "/tags/0").id(), first_id);
    assert_eq!(input_with_binding(&root, "/tags/2").id(), second_id);

    let remove = array
        .query_selector(&format!("button[id='{inserted_id}-remove']"))
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    remove.focus().unwrap();
    remove.click();
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({ "tags": ["same", "same"] })
    );
    let next = poll_dom(|| {
        let input = root
            .query_selector("input[name='/tags/1']")
            .ok()
            .flatten()?
            .dyn_into::<HtmlInputElement>()
            .ok()?;
        (input.id() == second_id).then_some(input)
    })
    .await;
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        (focused.id() == next.id()).then_some(())
    })
    .await;
    poll_dom(|| {
        array
            .query_selector("[data-array-status]")
            .ok()
            .flatten()?
            .text_content()?
            .contains("removed")
            .then_some(())
    })
    .await;
    assert_eq!(
        status.text_content().as_deref(),
        Some("Tags item removed from position 2.")
    );
    assert!(second.is_same_node(Some(&next)));
    assert!(
        form_handle
            .node(second_identity)
            .unwrap()
            .unwrap()
            .read()
            .unwrap()
            .unwrap()
            .touched
    );

    let append = array
        .query_selector("button[data-append-item]")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    append.focus().unwrap();
    append.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "tags": ["same", "same", "valid"] }))
        .then_some(())
    })
    .await;
    let appended = poll_dom(|| {
        root.query_selector("input[name='/tags/2']")
            .ok()
            .flatten()?
            .dyn_into::<HtmlInputElement>()
            .ok()
    })
    .await;
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        (focused.id() == appended.id()).then_some(())
    })
    .await;
    poll_dom(|| {
        array
            .query_selector("[data-array-status]")
            .ok()
            .flatten()?
            .text_content()?
            .contains("added")
            .then_some(())
    })
    .await;
    assert_eq!(
        status.text_content().as_deref(),
        Some("Tags item added at position 3.")
    );
    let current_first = root
        .query_selector(&format!("input[id='{first_id}']"))
        .unwrap()
        .unwrap()
        .dyn_into::<HtmlInputElement>()
        .unwrap();
    let current_second = root
        .query_selector(&format!("input[id='{second_id}']"))
        .unwrap()
        .unwrap()
        .dyn_into::<HtmlInputElement>()
        .unwrap();
    assert!(first.is_same_node(Some(&current_first)));
    assert!(second.is_same_node(Some(&current_second)));

    let remove_first = array
        .query_selector(&format!("button[id='{first_id}-remove']"))
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    remove_first.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "tags": ["same", "valid"] }))
        .then_some(())
    })
    .await;
    let first_removal_status = poll_dom(|| {
        let status = array.query_selector("[data-array-status]").ok().flatten()?;
        (status.text_content()?.contains("removed from position 1")).then_some(status)
    })
    .await;
    let first_removal_sequence = first_removal_status
        .get_attribute("data-announcement-sequence")
        .unwrap();
    assert!(
        form_handle
            .node(second_identity)
            .unwrap()
            .unwrap()
            .read()
            .unwrap()
            .unwrap()
            .touched
    );
    let remove_first = array
        .query_selector(&format!("button[id='{second_id}-remove']"))
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    remove_first.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "tags": ["valid"] }))
        .then_some(())
    })
    .await;
    let repeated_removal_status = poll_dom(|| {
        let status = array.query_selector("[data-array-status]").ok().flatten()?;
        (status.text_content()?.contains("removed from position 1")
            && status
                .get_attribute("data-announcement-sequence")
                .as_deref()
                != Some(first_removal_sequence.as_str()))
        .then_some(status)
    })
    .await;
    assert_ne!(
        repeated_removal_status.get_attribute("data-announcement-sequence"),
        Some(first_removal_sequence)
    );

    let post_baseline = input_with_binding(&root, "/tags/0");
    let post_baseline_id = post_baseline.id();
    post_baseline.focus().unwrap();
    form_handle
        .reset()
        .expect("the array form should reset without a borrow conflict");
    let restored = poll_dom(|| {
        if form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            != json!({ "tags": ["same", "same"] })
        {
            return None;
        }
        let first = maybe_input_with_binding(&root, "/tags/0")?;
        let second = maybe_input_with_binding(&root, "/tags/1")?;
        (first.id() == first_id && second.id() == second_id).then_some((first, second))
    })
    .await;
    let focused = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element()
        .unwrap();
    assert_ne!(focused.id(), post_baseline_id);
    restored.1.focus().unwrap();
    form_handle
        .reinitialize(json!({ "tags": ["same", "same"] }))
        .unwrap();
    poll_dom(|| {
        let first = maybe_input_with_binding(&root, "/tags/0")?;
        let second = maybe_input_with_binding(&root, "/tags/1")?;
        (first.id() != first_id && second.id() != second_id).then_some(())
    })
    .await;
    let focused = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element()
        .unwrap();
    assert_ne!(focused.id(), second_id);

    root.remove();
}

#[wasm_bindgen_test]
async fn fixed_object_array_rows_edit_focus_localize_and_submit_in_the_browser() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(fixed_object_array_test_app).await;
    let array = poll_dom(|| {
        root.query_selector("fieldset[data-schemaform-array]")
            .expect("the fixed-object array selector should be valid")
    })
    .await;
    accessibility_checkpoint(
        "array-object",
        "fixed_object_array_rows_edit_focus_localize_and_submit_in_the_browser",
        &root,
    )
    .await;
    let array_identity = control_with_binding(&form_handle, "/people");
    let array_projection = form_handle
        .node(array_identity)
        .expect("the array node should be readable")
        .expect("the array node should remain present")
        .read()
        .expect("the array projection should be readable")
        .expect("the array projection should remain present");
    assert_eq!(array_projection.collection_items.len(), 2);
    let first_row = array_projection.collection_items[0].identity;
    let row_projection = form_handle
        .node(first_row)
        .expect("the item root should be readable")
        .expect("the item root should remain present")
        .read()
        .expect("the item projection should be readable")
        .expect("the item projection should remain present");
    assert!(
        row_projection.collection_items.is_empty(),
        "item-template descendants are not direct collection items"
    );
    let rows = array.query_selector_all("[data-array-item]").unwrap();
    assert_eq!(rows.length(), 2);
    let first_city = input_with_binding(&root, "/people/0/address/city");
    let second_city = input_with_binding(&root, "/people/1/address/city");
    let first_city_node: web_sys::Node = first_city.clone().dyn_into().unwrap();
    let second_city_node: web_sys::Node = second_city.clone().dyn_into().unwrap();
    assert_ne!(first_city.id(), second_city.id());
    let second_row = rows.get(1).unwrap().dyn_into::<web_sys::Element>().unwrap();
    let legends = second_row.query_selector_all("legend").unwrap();
    assert_eq!(
        legends.get(0).unwrap().text_content().as_deref(),
        Some("Person")
    );
    assert_eq!(
        legends.get(1).unwrap().text_content().as_deref(),
        Some("Address")
    );
    assert!(
        second_row
            .text_content()
            .unwrap()
            .contains("City fallback help.")
    );

    second_city.focus().unwrap();
    second_city.blur().unwrap();
    let city_identity = control_with_binding(&form_handle, "/people/1/address/city");
    form_handle
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            form_handle
                .reader()
                .read()
                .expect("form should be readable")
                .data_revision,
            [ExternalFinding::blocking(
                "review-city",
                JsonPointer::parse("/people/1/address/city").unwrap(),
                json!({}),
            )],
        ))
        .unwrap();
    poll_dom(|| {
        second_row
            .query_selector("[data-external-finding='review-city']")
            .ok()
            .flatten()
    })
    .await;
    assert!(
        second_row
            .text_content()
            .unwrap()
            .contains("server reported review-city.")
    );

    let move_up = second_row
        .query_selector("button[data-move-item-up]")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    move_up.focus().unwrap();
    move_up.click();
    let moved_city = poll_dom(|| {
        let input = input_with_binding(&root, "/people/0/address/city");
        let node: web_sys::Node = input.clone().dyn_into().ok()?;
        second_city_node.is_same_node(Some(&node)).then_some(input)
    })
    .await;
    assert_eq!(
        control_with_binding(&form_handle, "/people/0/address/city"),
        city_identity
    );
    let moved_row = moved_city.closest("[data-array-item]").unwrap().unwrap();
    let move_down = moved_row
        .query_selector("button[data-move-item-down]")
        .unwrap()
        .expect("the first fixed-object row should move down")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        (focused.id() == move_down.id()).then_some(())
    })
    .await;
    move_down.click();
    let moved_down_city = poll_dom(|| {
        let input = input_with_binding(&root, "/people/1/address/city");
        let node: web_sys::Node = input.clone().dyn_into().ok()?;
        second_city_node.is_same_node(Some(&node)).then_some(input)
    })
    .await;
    let moved_down_row = moved_down_city
        .closest("[data-array-item]")
        .unwrap()
        .unwrap();
    let move_back_up = moved_down_row
        .query_selector("button[data-move-item-up]")
        .unwrap()
        .expect("the last fixed-object row should move up")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        (focused.id() == move_back_up.id()).then_some(())
    })
    .await;
    assert!(
        array
            .query_selector("[data-array-status]")
            .unwrap()
            .unwrap()
            .text_content()
            .unwrap()
            .contains("moved down")
    );
    move_back_up.click();
    let moved_city = poll_dom(|| {
        let input = input_with_binding(&root, "/people/0/address/city");
        let node: web_sys::Node = input.clone().dyn_into().ok()?;
        second_city_node.is_same_node(Some(&node)).then_some(input)
    })
    .await;
    assert!(
        form_handle
            .node(city_identity)
            .unwrap()
            .unwrap()
            .read()
            .unwrap()
            .unwrap()
            .touched
    );
    assert!(
        moved_city
            .parent_element()
            .unwrap()
            .text_content()
            .unwrap()
            .contains("server reported review-city.")
    );

    dispatch_input(&moved_city, "Li");
    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    dispatch_submit(&form);
    let summary_action = poll_dom(|| {
        root.query_selector("[data-finding-summary] [data-finding='minLength'] button")
            .ok()
            .flatten()?
            .dyn_into::<web_sys::HtmlElement>()
            .ok()
    })
    .await;
    summary_action.click();
    wait_for_input_focus(&moved_city, "summary").await;
    assert!(submitted.borrow().is_none());

    dispatch_input(&moved_city, "Paris");
    let first_row = array
        .query_selector_all("[data-array-item]")
        .unwrap()
        .get(0)
        .unwrap()
        .dyn_into::<web_sys::Element>()
        .unwrap();
    first_row
        .query_selector("button[data-insert-item-before]")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    let inserted = poll_dom(|| {
        let input = input_with_binding(&root, "/people/0/name");
        (input.value() == "New").then_some(input)
    })
    .await;
    let inserted_city = input_with_binding(&root, "/people/0/address/city");
    wait_for_input_focus(&inserted_city, "insert").await;
    let moved_after_insert = input_with_binding(&root, "/people/1/address/city");
    let moved_after_insert_node: web_sys::Node = moved_after_insert.clone().dyn_into().unwrap();
    assert!(second_city_node.is_same_node(Some(&moved_after_insert_node)));
    let inserted_row = inserted.closest("[data-array-item]").unwrap().unwrap();
    let remove_inserted = inserted_row
        .query_selector("button[data-remove-item]")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap();
    remove_inserted.focus().unwrap();
    remove_inserted.click();
    poll_dom(|| (array.query_selector_all("[data-array-item]").ok()?.length() == 2).then_some(()))
        .await;
    let moved_after_remove = input_with_binding(&root, "/people/0/address/city");
    let moved_after_remove_node: web_sys::Node = moved_after_remove.clone().dyn_into().unwrap();
    assert!(second_city_node.is_same_node(Some(&moved_after_remove_node)));
    wait_for_input_focus(&moved_after_remove, "remove").await;
    array
        .query_selector("button[data-append-item]")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    poll_dom(|| {
        let input = root
            .query_selector("input[name='/people/2/name']")
            .ok()
            .flatten()?
            .dyn_into::<HtmlInputElement>()
            .ok()?;
        (input.value() == "New").then_some(())
    })
    .await;
    let appended_city = poll_dom(|| {
        root.query_selector("input[name='/people/2/address/city']")
            .ok()
            .flatten()?
            .dyn_into::<HtmlInputElement>()
            .ok()
    })
    .await;
    wait_for_input_focus(&appended_city, "append").await;
    let current_moved: web_sys::Node = input_with_binding(&root, "/people/0/address/city")
        .dyn_into()
        .unwrap();
    let current_first: web_sys::Node = input_with_binding(&root, "/people/1/address/city")
        .dyn_into()
        .unwrap();
    assert!(second_city_node.is_same_node(Some(&current_moved)));
    assert!(first_city_node.is_same_node(Some(&current_first)));

    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        snapshot.form_data(),
        &json!({
            "people": [
                { "name": "Ada", "address": { "city": "Paris" } },
                { "name": "Ada", "address": { "city": "Rome" } },
                { "name": "New", "address": { "city": "Seed" } }
            ]
        })
    );

    form_handle
        .try_transact(|draft| {
            draft.set(&JsonPointer::parse("/people/0").unwrap(), json!(7));
            Ok::<_, ()>(())
        })
        .unwrap();
    let replace = poll_dom(|| {
        array
            .query_selector("[data-array-item] button[data-replace-value]")
            .ok()
            .flatten()?
            .dyn_into::<web_sys::HtmlElement>()
            .ok()
    })
    .await;
    replace.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")["people"][0]
            == json!({ "name": "New", "address": { "city": "Seed" } }))
        .then_some(())
    })
    .await;
    assert_eq!(
        snapshot.form_data()["people"][0],
        json!({ "name": "Ada", "address": { "city": "Paris" } })
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn authored_array_rows_render_edit_focus_and_submit_in_the_browser() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(authored_array_test_app).await;
    let array = poll_dom(|| {
        root.query_selector("fieldset[data-schemaform-array]")
            .ok()
            .flatten()
    })
    .await;
    let rows = array.query_selector_all("[data-array-item]").unwrap();
    assert_eq!(rows.length(), 2);
    let second_row = rows.get(1).unwrap().dyn_into::<web_sys::Element>().unwrap();
    assert!(
        second_row
            .text_content()
            .unwrap()
            .contains("Person details")
    );
    assert_eq!(
        second_row
            .query_selector("fieldset.schemaform-authored-group > legend")
            .unwrap()
            .and_then(|legend| legend.text_content())
            .as_deref(),
        Some("Location")
    );

    let first_city = input_with_binding(&root, "/people/0/address/city");
    let second_city = input_with_binding(&root, "/people/1/address/city");
    assert_ne!(first_city.id(), second_city.id());
    dispatch_input(&second_city, "Li");
    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    dispatch_submit(&form);
    let summary_action = poll_dom(|| {
        root.query_selector("[data-finding-summary] [data-finding='minLength'] button")
            .ok()
            .flatten()?
            .dyn_into::<web_sys::HtmlElement>()
            .ok()
    })
    .await;
    summary_action.click();
    wait_for_input_focus(&second_city, "authored array summary").await;
    assert!(submitted.borrow().is_none());

    dispatch_input(&second_city, "Paris");
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        snapshot.form_data(),
        &json!({
            "people": [
                { "name": "Ada", "address": { "city": "Rome" } },
                { "name": "Ada", "address": { "city": "Paris" } }
            ]
        })
    );

    *submitted.borrow_mut() = None;
    form_handle
        .try_transact(|draft| {
            draft.set(&JsonPointer::parse("/people/1").unwrap(), json!({}));
            Ok::<_, ()>(())
        })
        .unwrap();
    dispatch_submit(&form);
    let required_action = poll_dom(|| {
        root.query_selector("[data-finding-summary] [data-finding='required'] button")
            .ok()
            .flatten()?
            .dyn_into::<web_sys::HtmlElement>()
            .ok()
    })
    .await;
    let row_group = second_row
        .query_selector("fieldset[data-schemaform-fixed-object]")
        .unwrap()
        .expect("the authored subtree should retain its item root");
    required_action.click();
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        (focused.id() == row_group.id()).then_some(())
    })
    .await;
    assert!(submitted.borrow().is_none());

    form_handle
        .try_transact(|draft| {
            draft.set(&JsonPointer::parse("/people/1").unwrap(), json!(7));
            Ok::<_, ()>(())
        })
        .unwrap();
    let replace = poll_dom(|| {
        second_row
            .query_selector("button[data-replace-value]")
            .ok()
            .flatten()?
            .dyn_into::<web_sys::HtmlElement>()
            .ok()
    })
    .await;
    replace.click();
    poll_dom(|| {
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable")["people"][1]
            .is_object()
            .then_some(())
    })
    .await;
    assert_eq!(
        snapshot.form_data()["people"][1],
        json!({ "name": "Ada", "address": { "city": "Paris" } })
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn authored_scalar_array_rows_edit_focus_and_submit_in_the_browser() {
    let MountedTestApp {
        root, submitted, ..
    } = mount_test_app(authored_scalar_array_test_app).await;
    let rows = root.query_selector_all("[data-array-item]").unwrap();
    assert_eq!(rows.length(), 2);
    assert_eq!(
        root.query_selector("button[data-append-item]")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Add Entry")
    );
    assert_eq!(
        rows.get(0)
            .unwrap()
            .dyn_into::<web_sys::Element>()
            .unwrap()
            .query_selector("button[data-remove-item]")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Remove Entry")
    );
    let first = input_with_binding(&root, "/tags/0");
    let second = input_with_binding(&root, "/tags/1");
    assert_ne!(first.id(), second.id());

    dispatch_input(&second, "Li");
    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    dispatch_submit(&form);
    let summary_action = poll_dom(|| {
        root.query_selector("[data-finding-summary] [data-finding='minLength'] button")
            .ok()
            .flatten()?
            .dyn_into::<web_sys::HtmlElement>()
            .ok()
    })
    .await;
    summary_action.click();
    wait_for_input_focus(&second, "authored scalar array summary").await;
    assert!(submitted.borrow().is_none());

    dispatch_input(&second, "Paris");
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &json!({ "tags": ["same", "Paris"] }));

    root.remove();
}

#[wasm_bindgen_test]
async fn duplicate_fixed_object_array_lifecycle_updates_dom_keys_by_item_identity() {
    let MountedTestApp {
        root, form_handle, ..
    } = mount_test_app(fixed_object_array_test_app).await;
    let array = poll_dom(|| {
        root.query_selector("fieldset[data-schemaform-array]")
            .ok()
            .flatten()
    })
    .await;
    let duplicate = json!({ "name": "Ada", "address": { "city": "Rome" } });
    let baseline_data = json!({ "people": [duplicate.clone(), duplicate.clone()] });
    let first = input_with_binding(&root, "/people/0/address/city");
    let second = input_with_binding(&root, "/people/1/address/city");
    let first_id = first.id();
    let second_id = second.id();
    let first_identity = control_with_binding(&form_handle, "/people/0/address/city");
    let second_identity = control_with_binding(&form_handle, "/people/1/address/city");
    let first_node: web_sys::Node = first.clone().dyn_into().unwrap();
    let second_node: web_sys::Node = second.clone().dyn_into().unwrap();
    second.focus().unwrap();
    second.blur().unwrap();
    poll_dom(|| {
        form_handle
            .node(second_identity)
            .ok()??
            .read()
            .ok()??
            .touched
            .then_some(())
    })
    .await;

    first
        .closest("[data-array-item]")
        .unwrap()
        .unwrap()
        .query_selector("button[data-remove-item]")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "people": [duplicate.clone()] }))
        .then_some(())
    })
    .await;
    let shifted = poll_dom(|| {
        let input = input_with_binding(&root, "/people/0/address/city");
        (input.id() == second_id).then_some(input)
    })
    .await;
    let shifted_node: web_sys::Node = shifted.clone().dyn_into().unwrap();
    assert_eq!(shifted.id(), second_id);
    assert_eq!(
        control_with_binding(&form_handle, "/people/0/address/city"),
        second_identity
    );
    assert!(second_node.is_same_node(Some(&shifted_node)));

    array
        .query_selector("button[data-append-item]")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({
                "people": [
                    duplicate.clone(),
                    { "name": "New", "address": { "city": "Seed" } }
                ]
            }))
        .then_some(())
    })
    .await;
    let appended_identity = control_with_binding(&form_handle, "/people/1/address/city");
    form_handle
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            form_handle
                .reader()
                .read()
                .expect("form should be readable")
                .data_revision,
            [ExternalFinding::blocking(
                "review-city",
                JsonPointer::parse("/people/0/address/city").unwrap(),
                json!({}),
            )],
        ))
        .unwrap();
    poll_dom(|| {
        root.query_selector("[data-external-finding='review-city']")
            .ok()
            .flatten()
    })
    .await;
    let before_reset = form_handle
        .node(second_identity)
        .unwrap()
        .unwrap()
        .read()
        .unwrap()
        .unwrap();
    assert!(before_reset.touched);
    assert_eq!(before_reset.external_findings.len(), 1);
    shifted.focus().unwrap();

    let reset = form_handle
        .reset()
        .expect("the fixed-object array should reset without a borrow conflict");

    let (restored_first, restored_second) = poll_dom(|| {
        if form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            != baseline_data
        {
            return None;
        }
        let first = maybe_input_with_binding(&root, "/people/0/address/city")?;
        let second = maybe_input_with_binding(&root, "/people/1/address/city")?;
        (first.id() == first_id && second.id() == second_id).then_some((first, second))
    })
    .await;
    let restored_first_node: web_sys::Node = restored_first.clone().dyn_into().unwrap();
    let restored_second_node: web_sys::Node = restored_second.clone().dyn_into().unwrap();
    assert_eq!(restored_first.id(), first_id);
    assert_eq!(restored_second.id(), second_id);
    assert_eq!(
        control_with_binding(&form_handle, "/people/0/address/city"),
        first_identity
    );
    assert_eq!(
        control_with_binding(&form_handle, "/people/1/address/city"),
        second_identity
    );
    assert!(!first_node.is_same_node(Some(&restored_first_node)));
    assert!(second_node.is_same_node(Some(&restored_second_node)));
    assert_focused(&restored_second);
    assert!(form_handle.node(appended_identity).unwrap().is_none());
    let reset_second = form_handle
        .node(second_identity)
        .unwrap()
        .unwrap()
        .read()
        .unwrap()
        .unwrap();
    assert!(!reset_second.touched);
    assert!(reset_second.external_findings.is_empty());
    assert!(
        reset
            .removed()
            .any(|identity| identity == appended_identity)
    );

    restored_second.focus().unwrap();
    let reinitialized = form_handle
        .reinitialize(baseline_data.clone())
        .expect("equal array data should start a fresh repeated topology");
    let fresh_first = poll_dom(|| {
        let input = maybe_input_with_binding(&root, "/people/0/address/city")?;
        (input.id() != first_id).then_some(input)
    })
    .await;
    let fresh_second = input_with_binding(&root, "/people/1/address/city");
    let fresh_first_identity = control_with_binding(&form_handle, "/people/0/address/city");
    let fresh_second_identity = control_with_binding(&form_handle, "/people/1/address/city");
    assert_ne!(fresh_second.id(), second_id);
    assert_ne!(fresh_first_identity, first_identity);
    assert_ne!(fresh_second_identity, second_identity);
    assert!(form_handle.node(first_identity).unwrap().is_none());
    assert!(form_handle.node(second_identity).unwrap().is_none());
    assert!(
        reinitialized
            .removed()
            .any(|identity| identity == first_identity)
    );
    assert!(
        reinitialized
            .removed()
            .any(|identity| identity == second_identity)
    );
    let focused = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element()
        .unwrap();
    assert_ne!(focused.id(), second_id);

    let fresh_first_id = fresh_first.id();
    let fresh_second_id = fresh_second.id();
    fresh_second.focus().unwrap();
    form_handle
        .try_transact(|draft| {
            draft.set(
                &JsonPointer::parse("/people").unwrap(),
                json!([
                    { "name": "Ada", "address": { "city": "Rome" } },
                    { "name": "Grace", "address": { "city": "Paris" } }
                ]),
            );
            Ok::<_, ()>(())
        })
        .unwrap();
    let replaced_first = poll_dom(|| {
        let input = maybe_input_with_binding(&root, "/people/0/address/city")?;
        (input.id() != fresh_first_id).then_some(input)
    })
    .await;
    let replaced_second = input_with_binding(&root, "/people/1/address/city");
    assert_ne!(replaced_first.id(), fresh_first_id);
    assert_ne!(replaced_second.id(), fresh_second_id);
    assert!(form_handle.node(fresh_first_identity).unwrap().is_none());
    assert!(form_handle.node(fresh_second_identity).unwrap().is_none());
    let focused = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .active_element()
        .unwrap();
    assert_ne!(focused.id(), fresh_second_id);

    root.remove();
}

#[wasm_bindgen_test]
async fn scalar_array_add_focuses_the_first_focusable_row_action() {
    let MountedTestApp {
        root, form_handle, ..
    } = mount_test_app(constant_array_test_app).await;
    let array = poll_dom(|| {
        root.query_selector("fieldset[data-schemaform-array]")
            .ok()
            .flatten()
    })
    .await;
    array
        .query_selector("button[data-append-item]")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "values": ["fixed", "fixed"] }))
        .then_some(())
    })
    .await;
    let outputs = poll_dom(|| {
        let outputs = array.query_selector_all("output").ok()?;
        (outputs.length() == 2).then_some(outputs)
    })
    .await;
    let added: web_sys::HtmlElement = outputs.get(1).unwrap().dyn_into().unwrap();
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        (focused.id() == format!("{}-insert-before", added.id())).then_some(())
    })
    .await;

    root.remove();
}

#[wasm_bindgen_test]
async fn generated_annotations_supply_accessible_text_without_browser_validation_or_mutation() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(annotation_test_app).await;
    let input = input_with_binding(&root, "/email");
    let alias = input_with_binding(&root, "/alias");
    let nickname = input_with_binding(&root, "/nickname");
    let expected = json!({ "email": "not an email or base64", "nickname": "Ada" });

    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        expected
    );
    assert_eq!(input.type_(), "text");
    assert!(input.get_attribute("pattern").is_none());
    assert!(input.check_validity());
    let input_id = input.id();
    let label = root
        .query_selector(&format!("label[for='{input_id}']"))
        .expect("the label selector should be valid")
        .expect("the title fallback should render as a label");
    assert_eq!(label.text_content().as_deref(), Some("Email address"));
    let help = root
        .query_selector(&format!("#{input_id}-help"))
        .expect("the help selector should be valid")
        .expect("the description fallback should render as help text");
    assert_eq!(
        help.text_content().as_deref(),
        Some("Where account notices are sent.")
    );
    assert_eq!(
        input.get_attribute("aria-describedby").as_deref(),
        Some(help.id().as_str())
    );
    assert_eq!(
        nickname
            .parent_element()
            .expect("the nickname should have a control container")
            .query_selector_all("[data-capability-finding='annotation.conflict']")
            .expect("the warning selector should be valid")
            .length(),
        2
    );
    let set_alias = alias
        .parent_element()
        .expect("the alias should have a control container")
        .query_selector("button[data-set-value]")
        .expect("the set-value selector should be valid")
        .expect("the absent alias should expose explicit creation")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the explicit creation action should be a button");
    set_alias.click();
    let seeded = json!({
        "alias": "schema seed",
        "email": "not an email or base64",
        "nickname": "Ada"
    });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == seeded)
            .then_some(())
    })
    .await;

    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    assert!(form.no_validate());
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &seeded);

    root.remove();
}

#[wasm_bindgen_test]
async fn builtins_present_read_only_and_write_only_data_without_losing_submission_values() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(annotation_authority_test_app).await;
    let read_only = root
        .query_selector("output[name='/profile/name'][data-read-only]")
        .expect("the read-only output selector should be valid")
        .expect("read-only data should render as noninteractive content");
    let secret = input_with_binding(&root, "/secret");
    let secret_count = input_with_binding(&root, "/secret_count");
    let secret_rate = input_with_binding(&root, "/secret_rate");
    let secret_enabled = select_with_binding(&root, "/secret_enabled");
    let secret_mode = select_with_binding(&root, "/secret_mode");
    let nested_secret = input_with_binding(&root, "/credentials/token");
    let secret_region = root
        .query_selector("output[name='/secret_region']")
        .expect("the write-only constant selector should be valid")
        .expect("the write-only constant should render without its value");

    assert_eq!(read_only.text_content().as_deref(), Some("Ada"));
    assert_eq!(read_only.get_attribute("tabindex").as_deref(), Some("-1"));
    assert!(
        root.query_selector("input[name='/profile/name']")
            .expect("the read-only input selector should be valid")
            .is_none()
    );
    assert_eq!(secret.type_(), "password");
    assert_eq!(secret.value(), "");
    assert!(!secret.required());
    assert!(secret.has_attribute("data-write-only-replacement"));
    for input in [&secret_count, &secret_rate, &nested_secret] {
        assert_eq!(input.type_(), "password");
        assert_eq!(input.value(), "");
        assert!(!input.required());
        assert!(input.has_attribute("data-write-only-replacement"));
    }
    assert_eq!(secret_enabled.value(), "");
    assert_eq!(secret_mode.value(), "");
    assert_eq!(secret_enabled.selected_index(), 0);
    assert_eq!(secret_mode.selected_index(), 0);
    assert!(!secret_enabled.required());
    assert!(!secret_mode.required());
    let secret_enabled_options = select_options(&secret_enabled);
    let secret_mode_options = select_options(&secret_mode);
    assert_eq!(
        secret_enabled_options,
        [
            ("".to_owned(), "Choose replacement".to_owned()),
            ("false".to_owned(), "False".to_owned()),
            ("true".to_owned(), "True".to_owned()),
        ]
    );
    assert_eq!(
        secret_mode_options,
        [
            ("".to_owned(), "Choose replacement".to_owned()),
            ("choice-0".to_owned(), "private".to_owned()),
            ("choice-1".to_owned(), "public".to_owned()),
        ]
    );
    assert_eq!(
        secret_region.text_content().as_deref(),
        Some("Value is set")
    );
    assert_eq!(
        root.query_selector(&format!("label[for='{}']", secret_region.id()))
            .unwrap()
            .expect("the write-only constant should retain its noninteractive label")
            .text_content()
            .as_deref(),
        Some("Secret region")
    );
    assert!(secret_region.get_attribute("aria-label").is_none());
    assert!(
        !root
            .text_content()
            .unwrap_or_default()
            .contains("existing secret")
    );
    assert!(
        !root
            .text_content()
            .unwrap_or_default()
            .contains("nested secret")
    );
    assert!(!root.text_content().unwrap_or_default().contains("EU"));
    accessibility_checkpoint(
        "read-write-only",
        "builtins_present_read_only_and_write_only_data_without_losing_submission_values",
        &root,
    )
    .await;

    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    let submit = form
        .query_selector("button[type='submit']")
        .unwrap()
        .expect("the schema form should render a submit button");
    assert_eq!(submit.text_content().as_deref(), Some("Submit"));
    dispatch_submit(&form);
    let initial_snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        initial_snapshot.form_data(),
        &json!({
            "profile": { "name": "Ada" },
            "secret": "existing secret",
            "secret_count": 7,
            "secret_rate": 1.5,
            "secret_enabled": false,
            "secret_mode": "private",
            "secret_region": "EU",
            "credentials": { "token": "nested secret" }
        })
    );

    dispatch_input(&secret, "user replacement");
    dispatch_input(&secret_count, "9");
    dispatch_input(&secret_rate, "2.5");
    dispatch_input(&nested_secret, "nested replacement");

    dispatch_select_change(&secret_enabled, "false");
    assert_eq!(secret_enabled.value(), "");
    assert_eq!(secret_enabled.selected_index(), 0);
    assert_eq!(select_options(&secret_enabled), secret_enabled_options);
    dispatch_select_change(&secret_mode, "choice-0");
    assert_eq!(secret_mode.value(), "");
    assert_eq!(secret_mode.selected_index(), 0);
    assert_eq!(select_options(&secret_mode), secret_mode_options);

    dispatch_select_change(&secret_enabled, "true");
    assert_eq!(secret_enabled.value(), "");
    assert_eq!(secret_enabled.selected_index(), 0);
    dispatch_select_change(&secret_mode, "choice-1");
    assert_eq!(secret_mode.value(), "");
    assert_eq!(secret_mode.selected_index(), 0);
    let replace_region = secret_region
        .parent_element()
        .expect("the constant should have a control container")
        .query_selector("button[data-replace-value]")
        .expect("the constant replacement selector should be valid")
        .expect("a write-only constant should offer explicit replacement")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the constant replacement should be a button");
    replace_region.click();
    poll_dom(|| {
        let data = form_handle
            .reader()
            .form_data()
            .expect("form should be readable");
        (data["secret"] == "user replacement"
            && data["secret_count"] == 9
            && data["secret_rate"] == 2.5
            && data["secret_enabled"] == true
            && data["secret_mode"] == "public"
            && data["credentials"]["token"] == "nested replacement")
            .then_some(())
    })
    .await;
    assert_eq!(select_options(&secret_enabled), secret_enabled_options);
    assert_eq!(select_options(&secret_mode), secret_mode_options);
    assert_eq!(secret_enabled.value(), "");
    assert_eq!(secret_mode.value(), "");
    assert_eq!(secret_enabled.selected_index(), 0);
    assert_eq!(secret_mode.selected_index(), 0);
    *submitted.borrow_mut() = None;
    dispatch_submit(&form);
    let replacement_snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        replacement_snapshot.form_data()["secret"],
        "user replacement"
    );
    assert_eq!(replacement_snapshot.form_data()["secret_count"], 9);
    assert_eq!(replacement_snapshot.form_data()["secret_rate"], 2.5);
    assert_eq!(replacement_snapshot.form_data()["secret_enabled"], true);
    assert_eq!(replacement_snapshot.form_data()["secret_mode"], "public");
    assert_eq!(replacement_snapshot.form_data()["secret_region"], "EU");
    assert_eq!(
        replacement_snapshot.form_data()["credentials"]["token"],
        "nested replacement"
    );

    let before_host = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    let name_identity = control_with_binding(&form_handle, "/profile/name");
    let secret_identity = control_with_binding(&form_handle, "/secret");
    let host_transition = form_handle
        .try_transact(|draft| {
            draft.set(
                &schemaform::JsonPointer::parse("/profile/name").unwrap(),
                json!("Grace"),
            );
            draft.set(
                &schemaform::JsonPointer::parse("/secret").unwrap(),
                json!({ "hidden": "incompatible secret" }),
            );
            Ok::<_, ()>(())
        })
        .expect("the host should replace annotated data atomically");
    let after_host = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(
        host_transition.before_data_revision(),
        before_host.data_revision
    );
    assert_eq!(
        host_transition.before_state_revision(),
        before_host.state_revision
    );
    assert_eq!(
        host_transition.after_data_revision(),
        after_host.data_revision
    );
    assert_eq!(
        host_transition.after_state_revision(),
        after_host.state_revision
    );
    assert_ne!(
        host_transition.before_data_revision(),
        host_transition.after_data_revision()
    );
    assert_ne!(
        host_transition.before_state_revision(),
        host_transition.after_state_revision()
    );
    let changed = host_transition.changed().collect::<Vec<_>>();
    assert!(changed.contains(&name_identity));
    assert!(changed.contains(&secret_identity));
    poll_dom(|| {
        (read_only.text_content().as_deref() == Some("Grace")
            && secret.value().is_empty()
            && secret.read_only())
        .then_some(())
    })
    .await;
    assert!(
        !root
            .text_content()
            .unwrap_or_default()
            .contains("incompatible secret")
    );
    assert!(secret.required());
    let repair_secret = secret
        .parent_element()
        .expect("the secret should have a control container")
        .query_selector("button[data-replace-value]")
        .expect("the secret replacement selector should be valid")
        .expect("incompatible write-only data should offer explicit replacement")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the secret replacement should be a button");
    repair_secret.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")["secret"]
            == "")
            .then_some(())
    })
    .await;

    form_handle
        .try_transact(|draft| {
            draft.set(
                &schemaform::JsonPointer::parse("/secret").unwrap(),
                json!("host replacement"),
            );
            Ok::<_, ()>(())
        })
        .expect("the host should install the final write-only value");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")["secret"]
            == "host replacement"
            && secret.value().is_empty())
        .then_some(())
    })
    .await;
    assert!(
        !root
            .text_content()
            .unwrap_or_default()
            .contains("host replacement")
    );

    *submitted.borrow_mut() = None;
    dispatch_submit(&form);
    let host_snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        host_snapshot.form_data(),
        &json!({
            "profile": { "name": "Grace" },
            "secret": "host replacement",
            "secret_count": 9,
            "secret_rate": 2.5,
            "secret_enabled": true,
            "secret_mode": "public",
            "secret_region": "EU",
            "credentials": { "token": "nested replacement" }
        })
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn scalar_presence_controls_repair_explicitly_without_render_time_mutation() {
    let MountedTestApp {
        root,
        form_handle,
        submitted: _,
    } = mount_test_app(scalar_presence_test_app).await;
    let input = input_with_binding(&root, "/value");
    let value = control_with_binding(&form_handle, "/value");
    let mounted = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({})
    );
    assert_eq!(
        input.get_attribute("data-value-state").as_deref(),
        Some("missing")
    );
    assert!(input_with_binding(&root, "/settings/enabled").disabled());
    assert!(select_with_binding(&root, "/settings/mode").disabled());
    accessibility_checkpoint(
        "presence-missing",
        "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
        &root,
    )
    .await;

    input
        .focus()
        .expect("the missing input should accept focus");
    next_microtask().await;
    assert_eq!(
        form_handle
            .reader()
            .read()
            .expect("form should be readable")
            .data_revision,
        mounted.data_revision
    );
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({})
    );
    input.blur().expect("the untouched input should blur");
    poll_dom(|| {
        form_handle
            .node(value)
            .ok()??
            .read()
            .ok()??
            .touched
            .then_some(())
    })
    .await;
    assert_eq!(
        form_handle
            .reader()
            .read()
            .expect("form should be readable")
            .data_revision,
        mounted.data_revision
    );
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({})
    );

    dispatch_input(&input, "");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "value": "" })
            && input.get_attribute("data-value-state").as_deref() == Some("empty"))
        .then_some(())
    })
    .await;
    accessibility_checkpoint(
        "presence-empty",
        "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
        &root,
    )
    .await;
    let set_null = root
        .query_selector("button[data-set-null]")
        .expect("the set-null selector should be valid")
        .expect("a nullable non-null value should expose set-null")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the set-null action should be a button");
    set_null.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "value": null })
            && input.get_attribute("data-value-state").as_deref() == Some("null"))
        .then_some(())
    })
    .await;
    accessibility_checkpoint(
        "presence-null",
        "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
        &root,
    )
    .await;

    let remove = root
        .query_selector("button[data-remove-value]")
        .expect("the remove selector should be valid")
        .expect("an optional present scalar should expose remove")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the remove action should be a button");
    remove.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({})
            && input.get_attribute("data-value-state").as_deref() == Some("missing"))
        .then_some(())
    })
    .await;

    form_handle
        .reinitialize(json!({ "value": 7 }))
        .expect("the browser form should preserve incompatible scalar data");
    poll_dom(|| {
        (input.get_attribute("data-value-state").as_deref() == Some("incompatible")).then_some(())
    })
    .await;
    assert!(input.read_only());
    let incompatible = root
        .query_selector("[data-incompatible-value]")
        .expect("the incompatible-value selector should be valid")
        .expect("incompatible canonical data should remain visible");
    assert_eq!(incompatible.text_content().as_deref(), Some("7"));
    accessibility_checkpoint(
        "presence-incompatible",
        "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
        &root,
    )
    .await;
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({ "value": 7 })
    );

    let replace = root
        .query_selector("button[data-replace-value]")
        .expect("the replace selector should be valid")
        .expect("incompatible data should expose explicit replacement")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the replace action should be a button");
    replace.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "value": "" })
            && input.get_attribute("data-value-state").as_deref() == Some("empty"))
        .then_some(())
    })
    .await;
    accessibility_checkpoint(
        "presence-compatible",
        "scalar_presence_controls_repair_explicitly_without_render_time_mutation",
        &root,
    )
    .await;

    root.remove();
}

#[wasm_bindgen_test]
async fn custom_renderer_presence_affordances_repair_nullable_and_optional_scalars_without_render_time_mutation()
 {
    let MountedTestApp {
        root,
        form_handle,
        submitted: _,
    } = mount_test_app(custom_renderer_presence_test_app).await;
    let value = input_with_binding(&root, "/value");
    let note = input_with_binding(&root, "/note");
    assert!(value.has_attribute("data-affordance-widget"));
    let mounted = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    let form_data = || {
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
    };

    // Mounting a custom renderer that places affordances must not touch form data.
    assert_eq!(form_data(), json!({}));
    assert_eq!(affordance_kinds(&root, &value), ["Set", "SetNull"]);
    assert_eq!(affordance_kinds(&root, &note), ["Set"]);
    assert_described_by_resolves(&note);
    value
        .focus()
        .expect("the missing input should accept focus");
    next_microtask().await;
    value.blur().expect("the untouched input should blur");
    next_microtask().await;
    assert_eq!(
        form_handle
            .reader()
            .read()
            .expect("form should be readable")
            .data_revision,
        mounted.data_revision
    );
    assert_eq!(form_data(), json!({}));

    // Optional, non-nullable scalar: set materializes the seed, then only remove is offered.
    affordance_button(&root, &note, "Set").click();
    poll_dom(|| (form_data() == json!({ "note": "" })).then_some(())).await;
    poll_dom(|| (affordance_kinds(&root, &note) == ["RemoveValue"]).then_some(())).await;
    assert_eq!(note.value(), "");
    affordance_button(&root, &note, "RemoveValue").click();
    poll_dom(|| (form_data() == json!({})).then_some(())).await;
    poll_dom(|| (affordance_kinds(&root, &note) == ["Set"]).then_some(())).await;

    // Nullable scalar: set null, then set to the seed, then remove.
    affordance_button(&root, &value, "SetNull").click();
    poll_dom(|| (form_data() == json!({ "value": null })).then_some(())).await;
    poll_dom(|| (affordance_kinds(&root, &value) == ["Set", "RemoveValue"]).then_some(())).await;
    affordance_button(&root, &value, "Set").click();
    poll_dom(|| (form_data() == json!({ "value": "" })).then_some(())).await;
    poll_dom(|| (affordance_kinds(&root, &value) == ["SetNull", "RemoveValue"]).then_some(()))
        .await;
    affordance_button(&root, &value, "RemoveValue").click();
    poll_dom(|| (form_data() == json!({})).then_some(())).await;
    poll_dom(|| (affordance_kinds(&root, &value) == ["Set", "SetNull"]).then_some(())).await;

    // Incompatible data is preserved until the renderer's replace affordance repairs it.
    form_handle
        .reinitialize(json!({ "value": 7 }))
        .expect("the browser form should preserve incompatible scalar data");
    poll_dom(|| {
        (affordance_kinds(&root, &value) == ["SetNull", "RemoveValue", "Replace"]).then_some(())
    })
    .await;
    assert!(value.read_only());
    assert_eq!(form_data(), json!({ "value": 7 }));
    affordance_button(&root, &value, "Replace").click();
    poll_dom(|| (form_data() == json!({ "value": "" })).then_some(())).await;
    poll_dom(|| (affordance_kinds(&root, &value) == ["SetNull", "RemoveValue"]).then_some(()))
        .await;
    assert_described_by_resolves(&value);

    root.remove();
}

#[wasm_bindgen_test]
async fn arbitrary_precision_integer_browser_trace_matches_the_core_facade() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(integer_test_app).await;
    let input: HtmlInputElement = poll_dom(|| {
        root.query_selector("form input[name='/quantity']")
            .expect("the input selector should be valid")
            .map(|element| element.dyn_into().expect("the control should be an input"))
    })
    .await;
    let quantity = control_with_binding(&form_handle, "/quantity");
    let baseline_data: serde_json::Value =
        serde_json::from_str(r#"{"quantity":184467440737095516160}"#)
            .expect("the baseline should parse");

    assert_eq!(input.value(), "184467440737095516160");
    assert_eq!(input.type_(), "text");
    assert_eq!(
        input
            .parent_element()
            .expect("the input should have a control container")
            .get_attribute("data-schemaform-control")
            .as_deref(),
        Some("integer")
    );
    assert_eq!(input.get_attribute("inputmode").as_deref(), Some("numeric"));
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        baseline_data
    );
    let before = form_handle
        .node(quantity)
        .expect("the form should be readable")
        .expect("the generated integer control should exist")
        .read()
        .expect("the generated integer control should be readable")
        .expect("the generated integer control should remain present");
    assert_eq!(before.edit_buffer, None);
    assert_eq!(before.parse_blocker, None);
    assert!(!before.dirty);
    assert!(!before.touched);

    input
        .focus()
        .expect("the integer input should accept focus");
    assert_focused(&input);
    dispatch_input(&input, "-");
    poll_dom(|| {
        let projection = form_handle.node(quantity).ok()??.read().ok()??;
        (projection.edit_buffer.as_deref() == Some("-")
            && projection.parse_blocker == Some(ParseBlockerKind::InvalidInteger))
        .then_some(())
    })
    .await;
    assert_eq!(input.value(), "-");
    assert_focused(&input);
    poll_dom(|| (input.get_attribute("aria-invalid").as_deref() == Some("true")).then_some(()))
        .await;
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        baseline_data
    );
    let blocked = form_handle
        .node(quantity)
        .expect("the form should be readable")
        .expect("the generated integer control should exist")
        .read()
        .expect("the generated integer control should be readable")
        .expect("the generated integer control should remain present");
    assert!(!blocked.dirty);
    assert!(!blocked.touched);

    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    next_microtask().await;
    assert!(submitted.borrow().is_none());
    wait_for_summary_focus(&root).await;
    accessibility_checkpoint(
        "integer-parse-blocked",
        "arbitrary_precision_integer_browser_trace_matches_the_core_facade",
        &root,
    )
    .await;

    dispatch_input(&input, "184467440737095516159e0");
    let invalid_data: serde_json::Value =
        serde_json::from_str(r#"{"quantity":184467440737095516159}"#)
            .expect("the below-minimum form data should parse");
    poll_dom(|| {
        let projection = form_handle.node(quantity).ok()??.read().ok()??;
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == invalid_data
            && projection.edit_buffer.as_deref() == Some("184467440737095516159e0")
            && projection.parse_blocker.is_none())
        .then_some(())
    })
    .await;
    assert_eq!(input.value(), "184467440737095516159e0");
    let invalid = form_handle
        .node(quantity)
        .expect("the form should be readable")
        .expect("the generated integer control should exist")
        .read()
        .expect("the generated integer control should be readable")
        .expect("the generated integer control should remain present");
    assert_eq!(invalid.validation_findings.len(), 1);
    assert_eq!(invalid.validation_findings[0].code(), "minimum");
    let direct_preparation = form_handle
        .prepare_submission()
        .expect("direct submission should not conflict with another handle borrow");
    match direct_preparation.outcome() {
        SubmissionOutcome::Blocked(blockers) => assert!(
            blockers
                .iter()
                .any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
        ),
        SubmissionOutcome::Ready(_) => panic!("schema-invalid form data must block submission"),
    }
    poll_dom(|| (input.get_attribute("aria-invalid").as_deref() == Some("true")).then_some(()))
        .await;
    let rendered_finding = poll_dom(|| {
        root.query_selector("[data-validation-finding='minimum']")
            .expect("the finding selector should be valid")
    })
    .await;
    assert_eq!(
        rendered_finding.text_content().as_deref(),
        Some("Value must be at least 184467440737095516160.")
    );
    assert_eq!(
        input.get_attribute("aria-describedby").as_deref(),
        Some(rendered_finding.id().as_str())
    );
    dispatch_submit(&form);
    next_microtask().await;
    assert!(submitted.borrow().is_none());
    wait_for_summary_focus(&root).await;
    input
        .focus()
        .expect("the integer input should accept focus again after blocked submission");
    dispatch_input(&input, "184467440737095516161e0");
    next_microtask().await;
    assert!(
        input.is_connected(),
        "the integer input should remain mounted after summary updates"
    );
    let current_input = input_with_binding(&root, "/quantity");
    assert!(
        input.is_same_node(Some(&current_input)),
        "summary updates should preserve the integer input DOM node"
    );
    let corrected_data: serde_json::Value =
        serde_json::from_str(r#"{"quantity":184467440737095516161}"#)
            .expect("the corrected form data should parse");
    poll_dom(|| {
        let projection = form_handle.node(quantity).ok()??.read().ok()??;
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == corrected_data
            && projection.edit_buffer.as_deref() == Some("184467440737095516161e0")
            && projection.parse_blocker.is_none()
            && projection.validation_findings.is_empty()
            && projection.dirty
            && projection.touched)
            .then_some(())
    })
    .await;
    assert_eq!(input.value(), "184467440737095516161e0");
    poll_dom(|| {
        (input_with_binding(&root, "/quantity")
            .get_attribute("aria-invalid")
            .as_deref()
            != Some("true"))
        .then_some(())
    })
    .await;

    input.blur().expect("the integer input should blur");
    poll_dom(|| {
        let projection = form_handle.node(quantity).ok()??.read().ok()??;
        (projection.edit_buffer.is_none() && projection.touched).then_some(())
    })
    .await;
    poll_dom(|| (input.value() == "184467440737095516161").then_some(())).await;
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        corrected_data
    );

    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &corrected_data);
    assert_eq!(
        serde_json::to_string(snapshot.form_data()).expect("the snapshot should serialize"),
        r#"{"quantity":184467440737095516161}"#
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn arbitrary_precision_decimal_browser_trace_matches_the_core_facade() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(number_test_app).await;
    let input = input_with_binding(&root, "/rate");
    let rate = control_with_binding(&form_handle, "/rate");
    let baseline_data: serde_json::Value =
        serde_json::from_str(r#"{"rate":0.1000000000000000000000000000000000000001}"#)
            .expect("the decimal baseline should parse");
    assert_eq!(input.value(), "0.1000000000000000000000000000000000000001");
    assert_eq!(input.type_(), "text");
    assert_eq!(input.get_attribute("inputmode").as_deref(), Some("decimal"));
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        baseline_data
    );

    input
        .focus()
        .expect("the decimal input should accept focus");
    dispatch_input(&input, "1.000000000000000000000000000000000000001e-1");
    poll_dom(|| {
        let projection = form_handle.node(rate).ok()??.read().ok()??;
        (projection.edit_buffer.as_deref() == Some("1.000000000000000000000000000000000000001e-1")
            && projection.parse_blocker.is_none()
            && !projection.dirty
            && form_handle
                .reader()
                .form_data()
                .expect("form should be readable")
                == baseline_data)
            .then_some(())
    })
    .await;
    assert_eq!(
        input.value(),
        "1.000000000000000000000000000000000000001e-1"
    );
    assert_focused(&input);

    dispatch_input(&input, "1e-");
    poll_dom(|| {
        let projection = form_handle.node(rate).ok()??.read().ok()??;
        (projection.edit_buffer.as_deref() == Some("1e-")
            && projection.parse_blocker == Some(ParseBlockerKind::InvalidNumber))
        .then_some(())
    })
    .await;
    assert_eq!(input.value(), "1e-");
    assert_focused(&input);
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        baseline_data
    );
    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    next_microtask().await;
    assert!(submitted.borrow().is_none());
    wait_for_summary_focus(&root).await;

    input
        .blur()
        .expect("the incomplete decimal input should blur");
    poll_dom(|| {
        let projection = form_handle.node(rate).ok()??.read().ok()??;
        (projection.edit_buffer.as_deref() == Some("1e-")
            && projection.parse_blocker == Some(ParseBlockerKind::InvalidNumber)
            && projection.touched
            && input.value() == "1e-")
            .then_some(())
    })
    .await;
    input
        .focus()
        .expect("the decimal input should accept focus again");

    dispatch_input(&input, "0.10000000000000000000000000000000000000009");
    let invalid_data: serde_json::Value =
        serde_json::from_str(r#"{"rate":0.10000000000000000000000000000000000000009}"#)
            .expect("the below-minimum decimal should parse");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == invalid_data
            && form_handle
                .node(rate)
                .ok()??
                .read()
                .ok()??
                .validation_findings
                .first()?
                .code()
                == "minimum")
            .then_some(())
    })
    .await;
    dispatch_submit(&form);
    next_microtask().await;
    assert!(submitted.borrow().is_none());
    wait_for_summary_focus(&root).await;
    accessibility_checkpoint(
        "number-validation-blocked",
        "arbitrary_precision_decimal_browser_trace_matches_the_core_facade",
        &root,
    )
    .await;

    input
        .focus()
        .expect("the decimal input should accept focus again after blocked submission");
    dispatch_input(&input, "0.10000000000000000000000000000000000000011");
    let corrected_data: serde_json::Value =
        serde_json::from_str(r#"{"rate":0.10000000000000000000000000000000000000011}"#)
            .expect("the corrected decimal should parse");
    poll_dom(|| {
        let projection = form_handle.node(rate).ok()??.read().ok()??;
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == corrected_data
            && projection.parse_blocker.is_none()
            && projection.validation_findings.is_empty()
            && projection.dirty)
            .then_some(())
    })
    .await;
    input.blur().expect("the decimal input should blur");
    poll_dom(|| {
        let projection = form_handle.node(rate).ok()??.read().ok()??;
        (projection.edit_buffer.is_none() && projection.touched).then_some(())
    })
    .await;

    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &corrected_data);
    assert_eq!(
        serde_json::to_string(snapshot.form_data()).expect("the snapshot should serialize"),
        r#"{"rate":0.10000000000000000000000000000000000000011}"#
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn boolean_and_scalar_constant_controls_preserve_native_semantics() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(boolean_constant_test_app).await;
    let checkbox = input_with_binding(&root, "/enabled");
    let baseline = json!({ "enabled": false, "region": "EU" });
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        baseline
    );
    let before_focus = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(checkbox.type_(), "checkbox");
    assert!(!checkbox.checked());
    assert!(!checkbox.required());
    assert_eq!(
        checkbox.get_attribute("aria-required").as_deref(),
        Some("true")
    );
    assert_eq!(
        checkbox.get_attribute("data-value-state").as_deref(),
        Some("compatible")
    );
    let checkbox_id = checkbox.id();
    assert_eq!(
        root.query_selector(&format!("label[for='{checkbox_id}']"))
            .expect("the label selector should be valid")
            .expect("the checkbox should have a label")
            .text_content()
            .as_deref(),
        Some("Enabled")
    );
    checkbox
        .focus()
        .expect("the boolean checkbox should accept focus");
    next_microtask().await;
    let after_focus = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(after_focus.data_revision, before_focus.data_revision);
    assert_eq!(after_focus.state_revision, before_focus.state_revision);
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        baseline
    );

    let output = root
        .query_selector("output[name='/region']")
        .expect("the output selector should be valid")
        .expect("the constant should render as output");
    assert_eq!(output.text_content().as_deref(), Some("EU"));
    assert_eq!(output.get_attribute("tabindex").as_deref(), Some("-1"));
    assert_eq!(
        output.get_attribute("data-value-state").as_deref(),
        Some("compatible")
    );
    accessibility_checkpoint(
        "boolean-constant",
        "boolean_and_scalar_constant_controls_preserve_native_semantics",
        &root,
    )
    .await;
    assert!(
        root.query_selector("input[name='/region'], select[name='/region']")
            .expect("the editable-control selector should be valid")
            .is_none()
    );
    let output_id = output.id();
    assert_eq!(
        root.query_selector(&format!("label[for='{output_id}']"))
            .expect("the output label selector should be valid")
            .expect("the constant should have a label")
            .text_content()
            .as_deref(),
        Some("Region")
    );

    dispatch_checkbox_input(&checkbox, true);
    let edited = json!({ "enabled": true, "region": "EU" });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == edited)
            .then_some(())
    })
    .await;
    assert!(checkbox.checked());
    let enabled = control_with_binding(&form_handle, "/enabled");
    assert!(
        form_handle
            .node(enabled)
            .expect("the form should be readable")
            .expect("the checkbox control should exist")
            .read()
            .expect("the checkbox control should be readable")
            .expect("the checkbox control should remain present")
            .dirty
    );

    dispatch_checkbox_input(&checkbox, false);
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == baseline)
            .then_some(())
    })
    .await;
    assert!(!checkbox.checked());

    form_handle
        .try_transact(|draft| {
            draft.set(
                &schemaform::JsonPointer::parse("/region")
                    .expect("the region pointer should be valid"),
                json!("US"),
            );
            Ok::<_, ()>(())
        })
        .expect("the host should be able to install an invalid constant value");
    poll_dom(|| (output.text_content().as_deref() == Some("US")).then_some(())).await;
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));
    poll_dom(|| (output.get_attribute("aria-invalid").as_deref() == Some("true")).then_some(()))
        .await;
    assert!(
        root.query_selector("[data-validation-finding='const']")
            .expect("the constant finding selector should be valid")
            .is_some()
    );
    assert!(submitted.borrow().is_none());
    let summary_action = root
        .query_selector("[data-finding-summary] [data-finding='const'] button")
        .expect("the constant summary selector should be valid")
        .expect("the invalid constant should have a summary focus action")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the summary focus action should be a button");
    summary_action.click();
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        (focused.id() == output.id()).then_some(())
    })
    .await;

    form_handle
        .try_transact(|draft| {
            draft.set(
                &schemaform::JsonPointer::parse("/region")
                    .expect("the region pointer should be valid"),
                json!("EU"),
            );
            Ok::<_, ()>(())
        })
        .expect("the host should be able to restore the constant value");
    poll_dom(|| {
        (output.text_content().as_deref() == Some("EU")
            && output.get_attribute("aria-invalid").as_deref() == Some("false"))
        .then_some(())
    })
    .await;

    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &baseline);

    form_handle
        .reinitialize(json!({ "enabled": "yes", "region": "EU" }))
        .expect("the browser trace should accept incompatible boolean data");
    poll_dom(|| {
        (checkbox.get_attribute("data-value-state").as_deref() == Some("incompatible"))
            .then_some(())
    })
    .await;
    assert!(!checkbox.checked());
    assert!(!checkbox.disabled());
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));
    let repair_false = root
        .query_selector("button[data-replace-value]")
        .expect("the boolean repair selector should be valid")
        .expect("incompatible boolean data should have an explicit false repair")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the repair control should be a button");
    repair_false.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "enabled": false, "region": "EU" })
            && checkbox.get_attribute("data-value-state").as_deref() == Some("compatible"))
        .then_some(())
    })
    .await;

    form_handle
        .reinitialize(json!({ "region": "EU" }))
        .expect("the browser trace should accept missing required boolean data");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "region": "EU" })
            && checkbox.get_attribute("data-value-state").as_deref() == Some("missing"))
        .then_some(())
    })
    .await;
    assert!(!checkbox.checked());
    assert!(!checkbox.disabled());
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));

    let repair_false = root
        .query_selector("button[data-set-value]")
        .expect("the boolean repair selector should be valid")
        .expect("a missing required boolean should have an explicit false repair")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the repair control should be a button");
    repair_false.click();
    let repaired = json!({ "enabled": false, "region": "EU" });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == repaired
            && checkbox.get_attribute("data-value-state").as_deref() == Some("compatible"))
        .then_some(())
    })
    .await;
    assert!(!checkbox.checked());
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Ready(_)
    ));
    assert_eq!(snapshot.form_data(), &baseline);

    root.remove();
}

#[wasm_bindgen_test]
async fn finite_scalar_choices_use_opaque_tokens_and_submit_exact_values() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(choice_test_app).await;
    let select = select_with_binding(&root, "/choice");
    let choice = control_with_binding(&form_handle, "/choice");
    let projection = form_handle
        .node(choice)
        .expect("the form should be readable")
        .expect("the choice control should exist")
        .read()
        .expect("the choice control should be readable")
        .expect("the choice control should remain present");
    let null = projection
        .choice_options
        .iter()
        .find(|option| option.value.is_null())
        .expect("the null option should exist");
    let string_true = projection
        .choice_options
        .iter()
        .find(|option| option.value == json!("true"))
        .expect("the string option should exist");
    let boolean_true = projection
        .choice_options
        .iter()
        .find(|option| option.value == json!(true))
        .expect("the boolean option should exist");
    assert_eq!(string_true.label, boolean_true.label);
    assert_ne!(string_true.identity, boolean_true.identity);
    assert_ne!(string_true.identity.as_str(), string_true.label);
    assert_ne!(
        boolean_true.identity.as_str(),
        boolean_true.value.to_string()
    );
    assert_eq!(select.value(), null.identity.as_str());
    assert_eq!(
        select.get_attribute("data-value-state").as_deref(),
        Some("null")
    );
    accessibility_checkpoint(
        "choice",
        "finite_scalar_choices_use_opaque_tokens_and_submit_exact_values",
        &root,
    )
    .await;

    let before_noop = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    dispatch_select_change(&select, null.identity.as_str());
    next_microtask().await;
    next_microtask().await;
    let after_noop = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(after_noop.data_revision, before_noop.data_revision);
    assert_eq!(after_noop.state_revision, before_noop.state_revision);

    dispatch_select_change(&select, string_true.identity.as_str());
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")["choice"]
            == json!("true"))
        .then_some(())
    })
    .await;
    assert!(
        form_handle
            .node(choice)
            .expect("the form should be readable")
            .expect("the choice should remain")
            .read()
            .expect("the choice should remain readable")
            .expect("the choice should remain present")
            .dirty
    );

    dispatch_select_change(&select, null.identity.as_str());
    poll_dom(|| {
        let projection = form_handle.node(choice).ok()??.read().ok()??;
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")["choice"]
            .is_null()
            && !projection.dirty)
            .then_some(())
    })
    .await;

    let decimal = form_handle
        .node(choice)
        .expect("the form should be readable")
        .expect("the choice should remain")
        .read()
        .expect("the choice should remain readable")
        .expect("the choice should remain present")
        .choice_options
        .into_iter()
        .find(|option| option.value.is_number())
        .expect("the exact decimal option should exist");
    dispatch_select_change(&select, decimal.identity.as_str());
    let expected: serde_json::Value = serde_json::from_str(
        r#"{"choice":1.0000000000000000000000000000000000000001,"nothing":null,"region":"EU"}"#,
    )
    .expect("the exact browser choice data should parse");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == expected)
            .then_some(())
    })
    .await;

    for (binding, text) in [("/nothing", "null"), ("/region", "EU")] {
        let output = root
            .query_selector(&format!("output[name='{binding}']"))
            .expect("the fixed output selector should be valid")
            .expect("the fixed value should render as output");
        assert_eq!(output.text_content().as_deref(), Some(text));
        assert!(
            root.query_selector(&format!(
                "input[name='{binding}'], select[name='{binding}']"
            ))
            .expect("the editable fixed-control selector should be valid")
            .is_none()
        );
    }

    form_handle
        .reinitialize(json!({ "choice": false, "nothing": null, "region": "EU" }))
        .expect("the browser trace should accept an invalid choice");
    poll_dom(|| {
        (select.value().is_empty()
            && select.get_attribute("data-value-state").as_deref() == Some("incompatible"))
        .then_some(())
    })
    .await;
    let incompatible_option = select
        .query_selector("option:checked")
        .expect("the selected option selector should be valid")
        .expect("incompatible data should have a selected placeholder");
    assert_eq!(incompatible_option.text_content().as_deref(), Some("false"));
    assert!(incompatible_option.get_attribute("hidden").is_none());
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));
    poll_dom(|| {
        (select.get_attribute("aria-invalid").as_deref() == Some("true")
            && root
                .query_selector("[data-validation-finding='enum']")
                .expect("the enum finding selector should be valid")
                .is_some())
        .then_some(())
    })
    .await;
    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    next_microtask().await;
    assert!(submitted.borrow().is_none());

    dispatch_select_change(&select, boolean_true.identity.as_str());
    let repaired = json!({ "choice": true, "nothing": null, "region": "EU" });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == repaired)
            .then_some(())
    })
    .await;

    dispatch_submit(&form);
    let callback_snapshot = poll_dom(|| submitted.borrow().clone()).await;
    let direct_snapshot = match form_handle
        .prepare_submission()
        .expect("submission should not conflict with another handle borrow")
        .outcome()
    {
        SubmissionOutcome::Ready(snapshot) => snapshot.clone(),
        SubmissionOutcome::Blocked(_) => panic!("the repaired browser choice should submit"),
    };
    assert_eq!(callback_snapshot.form_data(), &repaired);
    assert_eq!(direct_snapshot.form_data(), callback_snapshot.form_data());

    root.remove();
}

#[wasm_bindgen_test]
async fn unsupported_one_of_region_is_presented_and_blocks_browser_submission() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(unsupported_one_of_test_app).await;
    let unsupported = root
        .query_selector("[data-schemaform-unsupported]")
        .expect("the unsupported-region selector should be valid")
        .expect("the unsupported region should render");
    assert_eq!(
        unsupported
            .get_attribute("data-capability-finding")
            .as_deref(),
        Some("applicator.one-of")
    );
    assert_eq!(
        unsupported.get_attribute("data-binding").as_deref(),
        Some("/contact")
    );
    assert!(
        unsupported
            .text_content()
            .is_some_and(|text| text.contains("Contact") && text.contains("oneOf"))
    );
    let described_by = unsupported
        .get_attribute("aria-describedby")
        .expect("the unsupported reason should be programmatically described");
    assert!(
        root.query_selector(&format!("#{described_by}"))
            .expect("the finding ID selector should be valid")
            .is_some()
    );
    accessibility_checkpoint(
        "unsupported",
        "unsupported_one_of_region_is_presented_and_blocks_browser_submission",
        &root,
    )
    .await;
    let name = input_with_binding(&root, "/name");
    dispatch_input(&name, "Grace");
    let edited = json!({ "contact": "ada@example.test", "name": "Grace" });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == edited)
            .then_some(())
    })
    .await;

    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Capability(finding)
                    if finding.code() == "applicator.one-of"
            ))
    ));
    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    next_microtask().await;
    assert!(submitted.borrow().is_none());

    root.remove();
}

#[wasm_bindgen_test]
async fn open_object_warning_is_accessible_and_does_not_block_browser_submission() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(open_object_test_app).await;
    let warning = root
        .query_selector(
            "[data-capability-finding='applicator.additional-properties.open'][data-blocking='false']",
        )
        .expect("the capability-warning selector should be valid")
        .expect("the open-object warning should render in the form summary");
    assert!(warning.text_content().is_some_and(|text| {
        text.contains("Undeclared properties are preserved and validated")
            && text.contains("arbitrary-key editing is unavailable")
    }));
    warning
        .query_selector("button")
        .expect("the warning focus-action selector should be valid")
        .expect("the summary warning should expose a focus action")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the focus action should be an HTML button")
        .click();
    next_microtask().await;
    let focused = web_sys::window()
        .expect("the browser should have a window")
        .document()
        .expect("the browser should have a document")
        .active_element()
        .expect("the summary focus action should focus the form");
    assert_eq!(focused.id(), warning.closest("form").unwrap().unwrap().id());

    let name = input_with_binding(&root, "/name");
    dispatch_input(&name, "Grace");
    let edited = json!({
        "name": "Grace",
        "hostowned": { "source": "import", "version": 7 }
    });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == edited)
            .then_some(())
    })
    .await;

    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &edited);

    *submitted.borrow_mut() = None;
    form_handle
        .try_transact(|draft| {
            draft.replace_all(json!({
                "name": "Grace",
                "HostOwned": { "source": "import", "version": 8 }
            }));
            Ok::<_, ()>(())
        })
        .expect("the host should be able to install schema-invalid undeclared data");
    dispatch_submit(&form);
    let validation_finding = poll_dom(|| {
        root.query_selector("[data-finding='propertyNames'][data-blocking='true']")
            .expect("the validation-summary selector should be valid")
    })
    .await;
    assert!(
        validation_finding
            .text_content()
            .is_some_and(|text| text.contains("propertyNames"))
    );
    next_microtask().await;
    assert!(submitted.borrow().is_none());

    root.remove();
}

#[wasm_bindgen_test]
async fn constrained_open_object_warnings_preserve_and_submit_undeclared_data() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(constrained_open_object_test_app).await;
    for (code, expected_text) in [
        (
            "applicator.additional-properties.schema-projection",
            "Schema-constrained additional properties are preserved and validated",
        ),
        (
            "applicator.pattern-properties.fixed-projection",
            "Pattern-matched properties are preserved and validated",
        ),
    ] {
        let warning = root
            .query_selector(&format!(
                "[data-capability-finding='{code}'][data-blocking='false']"
            ))
            .expect("the capability-warning selector should be valid")
            .unwrap_or_else(|| panic!("the browser summary should expose {code}"));
        assert!(
            warning
                .text_content()
                .is_some_and(|text| text.contains(expected_text)),
            "the warning should explain the fixed projection"
        );
    }

    let expected = json!({ "name": "Ada", "x-score": 7, "hostowned": 2 });
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        expected
    );
    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &expected);

    *submitted.borrow_mut() = None;
    form_handle
        .try_transact(|draft| {
            draft.set(
                &JsonPointer::parse("/hostowned").expect("the fixture pointer should be valid"),
                json!("invalid"),
            );
            draft.set(
                &JsonPointer::parse("/x-score").expect("the fixture pointer should be valid"),
                json!(-1),
            );
            Ok::<_, ()>(())
        })
        .expect("the host should be able to install invalid undeclared data");
    dispatch_submit(&form);
    poll_dom(|| {
        let has_type = root
            .query_selector("[data-finding='type'][data-blocking='true']")
            .expect("the validation selector should be valid")
            .is_some();
        let has_minimum = root
            .query_selector("[data-finding='minimum'][data-blocking='true']")
            .expect("the validation selector should be valid")
            .is_some();
        (has_type && has_minimum).then_some(())
    })
    .await;
    assert!(submitted.borrow().is_none());
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable")["hostowned"],
        json!("invalid")
    );
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable")["x-score"],
        json!(-1)
    );

    root.remove();
}

#[cfg(schemaform_test_validation_faults)]
#[wasm_bindgen_test]
async fn indeterminate_validation_is_presented_and_blocks_browser_submission() {
    let MountedTestApp {
        root, submitted, ..
    } = mount_test_app(indeterminate_test_app).await;
    assert!(
        root.query_selector("[data-finding='injected-validator-failure']")
            .expect("the indeterminate-finding selector should be valid")
            .is_none(),
        "indeterminate validation should follow submission visibility"
    );

    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    let finding = poll_dom(|| {
        root.query_selector("[data-finding='injected-validator-failure'][data-blocking='true']")
            .expect("the indeterminate-finding selector should be valid")
    })
    .await;
    assert!(
        finding
            .text_content()
            .is_some_and(|text| { text.contains("Validation could not be completed reliably") })
    );
    assert!(submitted.borrow().is_none());
    accessibility_checkpoint(
        "indeterminate-blocked",
        "indeterminate_validation_is_presented_and_blocks_browser_submission",
        &root,
    )
    .await;

    root.remove();
}

#[wasm_bindgen_test]
async fn nested_local_reference_edits_validates_and_submits_in_the_browser() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(nested_fixed_object_test_app).await;
    let input = input_with_binding(&root, "/contact/name");
    assert_eq!(input.value(), "Ada");
    assert!(input.required());
    let group = root
        .query_selector("fieldset[data-schemaform-fixed-object]")
        .expect("the fixed-object selector should be valid")
        .expect("the nested fixed object should render as a semantic group");
    let summary_action = root
        .query_selector("[data-capability-finding='applicator.additional-properties.open'] button")
        .expect("the nested warning summary-action selector should be valid")
        .expect("the nested warning should have a summary focus action")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the nested warning summary action should be an HTML button");
    summary_action.click();
    next_microtask().await;
    let focused = web_sys::window()
        .expect("the browser should have a window")
        .document()
        .expect("the browser should have a document")
        .active_element()
        .expect("the nested warning summary action should focus its group");
    assert_eq!(focused.id(), group.id());
    assert_eq!(
        group
            .query_selector("legend")
            .expect("the legend selector should be valid")
            .expect("the fixed-object group should have a legend")
            .text_content()
            .as_deref(),
        Some("Contact")
    );

    dispatch_input(&input, "Li");
    let invalid_data = json!({ "contact": { "name": "Li" } });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == invalid_data)
            .then_some(())
    })
    .await;
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));
    poll_dom(|| {
        root.query_selector("[data-validation-finding='minLength']")
            .expect("the finding selector should be valid")
    })
    .await;

    dispatch_input(&input, "Grace");
    let corrected_data = json!({ "contact": { "name": "Grace" } });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == corrected_data)
            .then_some(())
    })
    .await;
    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &corrected_data);

    form_handle
        .try_transact(|draft| {
            draft.replace_all(json!({ "contact": {} }));
            Ok::<_, ()>(())
        })
        .expect("the host should be able to install incomplete nested data");
    dispatch_submit(&form);
    let group_finding = poll_dom(|| {
        group
            .query_selector("[data-validation-finding='required']")
            .expect("the group finding selector should be valid")
    })
    .await;
    let group_warning = group
        .query_selector("[data-capability-finding='applicator.additional-properties.open']")
        .expect("the group warning selector should be valid")
        .expect("the nested open-object warning should be presented locally");
    assert_eq!(group.get_attribute("aria-invalid").as_deref(), Some("true"));
    assert_eq!(
        group.get_attribute("aria-describedby").as_deref(),
        Some(format!("{} {}", group_finding.id(), group_warning.id()).as_str())
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn optional_fixed_object_materializes_repairs_removes_and_submits_in_the_browser() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(optional_fixed_object_test_app).await;
    let settings = control_with_binding(&form_handle, "/settings");
    let name = control_with_binding(&form_handle, "/settings/name");
    let input = input_with_binding(&root, "/settings/name");
    let initial = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    let group = root
        .query_selector("fieldset[data-schemaform-fixed-object]")
        .expect("the fixed-object selector should be valid")
        .expect("the absent fixed object should still render its semantic group");
    assert_eq!(
        group
            .query_selector("legend")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Settings")
    );
    accessibility_checkpoint(
        "fixed-object",
        "optional_fixed_object_materializes_repairs_removes_and_submits_in_the_browser",
        &root,
    )
    .await;

    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({})
    );
    assert!(input.read_only());
    assert!(!input.required());
    assert!(
        root.query_selector("button[data-remove-value]")
            .unwrap()
            .is_none()
    );
    let materialize = root
        .query_selector("button[data-materialize]")
        .expect("the materialize selector should be valid")
        .expect("an absent fixed object should expose explicit materialization")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the materialize action should be a button");
    assert_eq!(
        form_handle
            .reader()
            .read()
            .expect("form should be readable")
            .data_revision,
        initial.data_revision
    );

    materialize.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "settings": { "name": "Li" } })
            && !input.read_only())
        .then_some(())
    })
    .await;
    assert!(input.required());
    assert!(
        form_handle
            .node(settings)
            .unwrap()
            .unwrap()
            .read()
            .unwrap()
            .unwrap()
            .dirty
    );
    assert!(
        form_handle
            .node(name)
            .unwrap()
            .unwrap()
            .read()
            .unwrap()
            .unwrap()
            .dirty
    );
    assert!(
        root.query_selector("button[data-materialize]")
            .unwrap()
            .is_none()
    );
    let remove = root
        .query_selector("button[data-remove-value]")
        .expect("the remove selector should be valid")
        .expect("a present optional fixed object should expose removal")
        .dyn_into::<web_sys::HtmlElement>()
        .expect("the remove action should be a button");

    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    let finding = poll_dom(|| {
        root.query_selector("[data-validation-finding='minLength']")
            .expect("the finding selector should be valid")
    })
    .await;
    assert!(submitted.borrow().is_none());
    assert_eq!(input.get_attribute("aria-invalid").as_deref(), Some("true"));
    assert_eq!(
        input.get_attribute("aria-describedby").as_deref(),
        Some(finding.id().as_str())
    );

    dispatch_input(&input, "Grace");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "settings": { "name": "Grace" } })
            && root
                .query_selector("[data-validation-finding='minLength']")
                .ok()
                .flatten()
                .is_none())
        .then_some(())
    })
    .await;
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(
        snapshot.form_data(),
        &json!({ "settings": { "name": "Grace" } })
    );

    remove.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({})
            && input.read_only())
        .then_some(())
    })
    .await;
    assert!(
        !form_handle
            .node(settings)
            .unwrap()
            .unwrap()
            .read()
            .unwrap()
            .unwrap()
            .dirty
    );
    assert!(
        !form_handle
            .node(name)
            .unwrap()
            .unwrap()
            .read()
            .unwrap()
            .unwrap()
            .dirty
    );
    assert!(
        root.query_selector("button[data-materialize]")
            .unwrap()
            .is_some()
    );
    assert!(
        root.query_selector("button[data-remove-value]")
            .unwrap()
            .is_none()
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn caller_supplied_anchored_resource_validates_and_submits_in_the_browser() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(anchored_resource_graph_test_app).await;
    let input = input_with_binding(&root, "/contact/name");
    assert_eq!(input.value(), "Ada");
    assert!(input.required());

    dispatch_input(&input, "Li");
    let invalid_data = json!({ "contact": { "name": "Li" } });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == invalid_data)
            .then_some(())
    })
    .await;
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));
    poll_dom(|| {
        root.query_selector("[data-validation-finding='minLength']")
            .expect("the finding selector should be valid")
    })
    .await;

    dispatch_input(&input, "Grace");
    let corrected_data = json!({ "contact": { "name": "Grace" } });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == corrected_data)
            .then_some(())
    })
    .await;
    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &corrected_data);

    root.remove();
}

#[wasm_bindgen_test]
async fn compatible_all_of_edits_validates_and_submits_in_the_browser() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(compatible_all_of_test_app).await;
    let input = input_with_binding(&root, "/name");
    assert_eq!(input.value(), "Ada");
    assert!(input.required());

    dispatch_input(&input, "Li");
    let invalid_data = json!({ "name": "Li" });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == invalid_data)
            .then_some(())
    })
    .await;
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(blocker, SubmissionBlocker::Validation(_)))
    ));
    poll_dom(|| {
        root.query_selector("[data-validation-finding='minLength']")
            .expect("the finding selector should be valid")
    })
    .await;

    dispatch_input(&input, "Grace");
    let corrected_data = json!({ "name": "Grace" });
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == corrected_data)
            .then_some(())
    })
    .await;
    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &corrected_data);

    root.remove();
}

#[wasm_bindgen_test]
async fn scalar_transition_rerenders_only_the_changed_instance_without_remounting() {
    let MountedIsolatedUpdatesTestApp {
        root,
        form_handle,
        lifecycle,
        matcher_calls,
    } = mount_isolated_updates_test_app().await;
    let first_name = control_with_binding(&form_handle, "/first_name");
    let last_name = control_with_binding(&form_handle, "/last_name");
    let first_input = input_with_binding(&root, "/first_name");
    let last_input = input_with_binding(&root, "/last_name");
    let first_node: web_sys::Node = first_input
        .clone()
        .dyn_into()
        .expect("the first-name input should be a DOM node");
    let last_node: web_sys::Node = last_input
        .clone()
        .dyn_into()
        .expect("the last-name input should be a DOM node");
    let before = lifecycle.borrow().clone();
    assert_eq!(*matcher_calls.borrow(), 2);
    assert_eq!(before.get(&first_name).map(|counts| counts.mounts), Some(1));
    assert_eq!(before.get(&last_name).map(|counts| counts.mounts), Some(1));
    assert_described_by_resolves(&first_input);
    assert_described_by_resolves(&last_input);

    dispatch_input(&first_input, "Grace");
    poll_dom(|| {
        let current = lifecycle.borrow();
        let previous = before.get(&first_name)?;
        (current.get(&first_name)?.renderer_calls == previous.renderer_calls + 1).then_some(())
    })
    .await;

    let after_edit = lifecycle.borrow().clone();
    assert_eq!(
        after_edit
            .get(&last_name)
            .map(|counts| counts.renderer_calls),
        before.get(&last_name).map(|counts| counts.renderer_calls)
    );
    assert_eq!(
        after_edit.get(&first_name).map(|counts| counts.mounts),
        Some(1)
    );
    assert_eq!(
        after_edit.get(&last_name).map(|counts| counts.mounts),
        Some(1)
    );
    let current_first = input_with_binding(&root, "/first_name");
    let current_last = input_with_binding(&root, "/last_name");
    let current_first_node: web_sys::Node = current_first
        .clone()
        .dyn_into()
        .expect("the current first-name input should be a DOM node");
    let current_last_node: web_sys::Node = current_last
        .clone()
        .dyn_into()
        .expect("the current last-name input should be a DOM node");
    assert!(first_node.is_same_node(Some(&current_first_node)));
    assert!(last_node.is_same_node(Some(&current_last_node)));
    assert_eq!(current_first.value(), "Grace");
    assert_eq!(current_last.value(), "Lovelace");

    dispatch_input(&current_first, "Grace");
    next_microtask().await;
    next_microtask().await;
    assert_eq!(*lifecycle.borrow(), after_edit);
    assert_eq!(
        *matcher_calls.borrow(),
        2,
        "runtime value and interaction changes must not rerun definition matchers"
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn ordinary_scalar_edit_updates_only_subscribed_displayed_state_generated() {
    assert_production_reactivity_gate("O50-generated-valid").await;
}

#[wasm_bindgen_test]
async fn ordinary_scalar_edit_updates_only_subscribed_displayed_state_authored() {
    assert_production_reactivity_gate("O50-authored-valid").await;
}

async fn assert_production_reactivity_gate(scenario_id: &str) {
    let scenario = browser_workload_pack::scenario_by_id(scenario_id)
        .unwrap_or_else(|| panic!("missing representative workload {scenario_id}"));
    let edited_binding = scenario
        .workloads
        .iter()
        .find_map(|workload| match workload {
            browser_workload_pack::Workload::Edit { binding, .. } => Some(binding.clone()),
            _ => None,
        })
        .expect("the representative workload should declare an edit binding");
    let MountedProductionReactivityTestApp {
        root,
        form_handle,
        observations,
        mut mounted,
    } = mount_production_reactivity_test_app(scenario).await;
    let edited_identity = control_with_binding(&form_handle, &edited_binding);
    let edited_input = input_with_binding(&root, &edited_binding);
    let initial = observations.borrow().clone();
    let mut mounted_nodes = HashMap::new();
    for observation in &initial {
        if observation.event == RenderEvent::Mounted {
            assert!(
                mounted_nodes
                    .insert(
                        observation.identity,
                        (observation.node_kind, observation.dom_id.clone()),
                    )
                    .is_none(),
                "{scenario_id} mounted one component scope per instance identity"
            );
        }
    }
    assert_eq!(
        mounted_nodes
            .values()
            .filter(|(kind, _)| *kind == RenderNodeKind::Control)
            .count(),
        50,
        "{scenario_id} must exercise the pre-calibration O50 semantic workload"
    );
    assert_eq!(
        mounted_nodes
            .values()
            .filter(|(kind, _)| *kind == RenderNodeKind::StaticLayout)
            .count(),
        if scenario_id.contains("-authored-") {
            1
        } else {
            3
        },
        "{scenario_id} must instrument every static layout in its render plan"
    );
    for identity in mounted_nodes.keys() {
        assert!(
            initial.iter().any(|observation| {
                observation.identity == *identity
                    && observation.event == RenderEvent::RendererEntered
            }),
            "{scenario_id} must record renderer entry for every mounted instance"
        );
    }
    let before_dom = mounted_nodes
        .iter()
        .map(|(identity, (_, dom_id))| {
            let node: web_sys::Node = root
                .owner_document()
                .unwrap()
                .get_element_by_id(dom_id)
                .unwrap_or_else(|| panic!("{scenario_id} should render keyed DOM id {dom_id}"))
                .into();
            (*identity, node)
        })
        .collect::<HashMap<_, _>>();
    let edited_dom_node: web_sys::Node = edited_input
        .clone()
        .dyn_into()
        .expect("the edited input should be a DOM node");
    observations.borrow_mut().clear();

    dispatch_input(&edited_input, "2");
    poll_dom(|| {
        observations
            .borrow()
            .iter()
            .any(|observation| {
                observation.identity == edited_identity
                    && observation.event == RenderEvent::RendererEntered
            })
            .then_some(())
    })
    .await;
    next_microtask().await;
    next_microtask().await;
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            .pointer(&edited_binding),
        Some(&json!(2)),
        "{scenario_id} must commit the ordinary scalar edit"
    );
    assert_eq!(
        input_with_binding(&root, &edited_binding).value(),
        "2",
        "{scenario_id} must display the committed scalar edit"
    );

    let after = observations.borrow().clone();
    for (identity, (node_kind, dom_id)) in &mounted_nodes {
        let events = after
            .iter()
            .filter(|observation| observation.identity == *identity)
            .collect::<Vec<_>>();
        assert!(
            events.iter().all(|observation| !matches!(
                observation.event,
                RenderEvent::Mounted | RenderEvent::Dropped
            )),
            "{scenario_id} must not remount {node_kind:?} {dom_id}"
        );
        let generated_required_ancestor_may_rerender = scenario_id.contains("-generated-")
            && *node_kind == RenderNodeKind::StaticLayout
            && before_dom[identity].contains(Some(&edited_dom_node));
        if *identity != edited_identity && !generated_required_ancestor_may_rerender {
            assert!(
                events.is_empty(),
                "{scenario_id} unrelated {node_kind:?} {dom_id} must record zero renders"
            );
        }
        let current: web_sys::Node = root
            .owner_document()
            .unwrap()
            .get_element_by_id(dom_id)
            .unwrap_or_else(|| panic!("{scenario_id} should preserve keyed DOM id {dom_id}"))
            .into();
        assert!(
            before_dom[identity].is_same_node(Some(&current)),
            "{scenario_id} must preserve keyed DOM identity for {node_kind:?} {dom_id}"
        );
    }
    assert_eq!(
        after
            .iter()
            .filter(|observation| {
                observation.identity == edited_identity
                    && observation.event == RenderEvent::RendererEntered
            })
            .count(),
        1,
        "{scenario_id} should enter only the edited production renderer"
    );

    observations.borrow_mut().clear();
    mounted.set(false);
    let dropped = poll_dom(|| {
        let mut dropped = HashMap::new();
        for observation in observations
            .borrow()
            .iter()
            .filter(|observation| observation.event == RenderEvent::Dropped)
        {
            *dropped.entry(observation.identity).or_insert(0_usize) += 1;
        }
        (dropped.len() == mounted_nodes.len()).then_some(dropped)
    })
    .await;
    assert_eq!(
        dropped
            .keys()
            .copied()
            .collect::<std::collections::HashSet<_>>(),
        mounted_nodes
            .keys()
            .copied()
            .collect::<std::collections::HashSet<_>>()
    );
    assert!(dropped.values().all(|count| *count == 1));

    root.remove();
}

#[wasm_bindgen_test]
async fn exact_widget_binding_is_definition_stable_and_grants_only_node_capabilities() {
    let MountedTestApp {
        root,
        form_handle,
        submitted: _,
    } = mount_test_app(exact_widget_test_app).await;
    let input: HtmlInputElement = root
        .query_selector("[data-exact-widget]")
        .unwrap()
        .expect("the exact renderer should provide its custom control")
        .dyn_into()
        .unwrap();
    assert_eq!(input.name(), "/name");
    assert!(input.required());
    assert!(!input.disabled());
    assert!(!input.read_only());
    assert_eq!(
        input.get_attribute("aria-invalid").as_deref(),
        Some("false")
    );
    assert_eq!(
        input.get_attribute("data-label-visible").as_deref(),
        Some("true")
    );
    assert_eq!(
        input.get_attribute("data-help").as_deref(),
        Some("Enter a full name")
    );
    assert_eq!(
        input.get_attribute("data-matcher-calls").as_deref(),
        Some("0")
    );
    assert_eq!(
        input.get_attribute("data-extension-count").as_deref(),
        Some("0")
    );
    assert_eq!(
        input.get_attribute("data-control-kind").as_deref(),
        Some("String")
    );
    assert_eq!(
        input.get_attribute("data-touched").as_deref(),
        Some("false")
    );
    assert_eq!(input.get_attribute("data-dirty").as_deref(), Some("false"));
    let help_ids = assert_described_by_resolves(&input);
    assert_eq!(
        help_ids.len(),
        1,
        "an untouched valid control is described only by its help"
    );
    let help = root
        .query_selector(&format!("#{}", help_ids[0]))
        .unwrap()
        .expect("the help id should resolve");
    assert!(
        help.has_attribute("data-exact-help"),
        "the custom renderer owns the help element"
    );
    assert_eq!(help.text_content().as_deref(), Some("Enter a full name"));
    assert_eq!(
        root.query_selector_all(".schemaform-help")
            .unwrap()
            .length(),
        0,
        "the adapter must not append its own help after a custom renderer"
    );

    dispatch_input(&input, "");
    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    dispatch_submit(&form);
    poll_dom(|| {
        (input_with_binding(&root, "/name")
            .get_attribute("aria-invalid")
            .as_deref()
            == Some("true"))
        .then_some(())
    })
    .await;
    let invalid_input = input_with_binding(&root, "/name");
    let referenced = assert_described_by_resolves(&invalid_input);
    assert_eq!(
        referenced.len(),
        2,
        "an invalid control is described by its help and its one finding"
    );
    let local_findings = root
        .query_selector_all("[data-validation-finding]")
        .unwrap();
    assert_eq!(
        local_findings.length(),
        1,
        "present_findings() renders the local finding exactly once"
    );
    let local_finding: web_sys::Element = local_findings.get(0).unwrap().dyn_into().unwrap();
    assert_eq!(local_finding.id(), referenced[1]);
    assert_eq!(
        invalid_input.get_attribute("data-dirty").as_deref(),
        Some("true")
    );
    dispatch_input(&input, "Grace");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "name": "Grace" }))
        .then_some(())
    })
    .await;
    let input = input_with_binding(&root, "/name");
    assert_eq!(
        input.get_attribute("data-matcher-calls").as_deref(),
        Some("0")
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn renderer_bind_preflight_reports_missing_exact_widgets_and_priority_ties() {
    let MountedTestApp { root, .. } = mount_test_app(renderer_bind_findings_test_app).await;
    assert!(
        root.query_selector("[data-bind-findings-complete]")
            .unwrap()
            .is_some()
    );
    assert!(
        root.query_selector("[data-schemaform-control]")
            .unwrap()
            .is_none()
    );
    root.remove();
}

#[wasm_bindgen_test]
async fn exact_uri_extensions_preflight_and_decorate_in_canonical_order() {
    let MountedTestApp {
        root,
        form_handle,
        submitted: _,
    } = mount_test_app(extension_preflight_test_app).await;
    let input: HtmlInputElement = root
        .query_selector("[data-extension='b'] > [data-extension='a'] [data-exact-widget]")
        .unwrap()
        .expect("the largest URI decorator should wrap the smallest URI decorator")
        .dyn_into()
        .unwrap();
    assert_eq!(
        input.get_attribute("data-extension-count").as_deref(),
        Some("2")
    );
    assert_eq!(
        root.query_selector_all(".schemaform-stack")
            .unwrap()
            .length(),
        1,
        "an ignored optional Auto extension must not add a semantic stack wrapper"
    );
    for marker in ["a", "b"] {
        assert_eq!(
            root.query_selector(&format!("[data-extension='{marker}']"))
                .unwrap()
                .unwrap()
                .get_attribute("data-preparation-count")
                .as_deref(),
            Some("4")
        );
    }

    dispatch_input(&input, "Grace");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "name": "Grace", "other": "kept" }))
        .then_some(())
    })
    .await;
    assert_eq!(
        root.query_selector("[data-extension='b']")
            .unwrap()
            .unwrap()
            .get_attribute("data-preparation-count")
            .as_deref(),
        Some("4"),
        "runtime state changes must not rerun definition-stable preparation"
    );
    root.remove();
}

#[wasm_bindgen_test]
async fn highest_static_matcher_wins_once_and_below_floor_leaves_the_builtin() {
    let MountedTestApp {
        root,
        form_handle,
        submitted: _,
    } = mount_test_app(matcher_priority_test_app).await;
    let high = input_with_binding(&root, "/high");
    let low = input_with_binding(&root, "/low");
    assert_eq!(
        high.get_attribute("data-priority-renderer").as_deref(),
        Some("highest")
    );
    assert_eq!(
        high.get_attribute("data-matcher-calls").as_deref(),
        Some("6")
    );
    assert!(low.get_attribute("data-priority-renderer").is_none());
    assert_described_by_resolves(&high);

    dispatch_input(&high, "Grace");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "high": "Grace", "low": "built in" }))
        .then_some(())
    })
    .await;
    assert_eq!(
        input_with_binding(&root, "/high")
            .get_attribute("data-matcher-calls")
            .as_deref(),
        Some("6")
    );
    root.remove();
}

#[wasm_bindgen_test]
async fn custom_item_widget_renders_under_adapter_owned_array() {
    let MountedTestApp {
        root,
        form_handle,
        submitted: _,
    } = mount_test_app(custom_array_item_widget_test_app).await;
    assert!(
        root.query_selector("[data-schemaform-array]")
            .unwrap()
            .is_some()
    );
    let first = root
        .query_selector("[data-exact-widget]")
        .unwrap()
        .expect("the first custom item control should render")
        .dyn_into::<HtmlInputElement>()
        .unwrap();
    assert_eq!(
        first.get_attribute("data-exact-widget").as_deref(),
        Some("")
    );
    assert_eq!(
        first.get_attribute("data-matcher-calls").as_deref(),
        Some("0")
    );
    assert_described_by_resolves(&first);

    dispatch_input(&first, "updated");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "tags": ["updated"] }))
        .then_some(())
    })
    .await;
    let append: web_sys::HtmlElement = root
        .query_selector("[data-append-item]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    append.click();
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "tags": ["updated", "new"] }))
        .then_some(())
    })
    .await;
    let custom_items = poll_dom(|| {
        let items = root.query_selector_all("[data-exact-widget]").ok()?;
        (items.length() == 2).then_some(items)
    })
    .await;
    let second = custom_items
        .get(1)
        .unwrap()
        .dyn_into::<HtmlInputElement>()
        .unwrap();
    assert_eq!(
        second.get_attribute("data-exact-widget").as_deref(),
        Some("")
    );
    assert_eq!(
        second.get_attribute("data-matcher-calls").as_deref(),
        Some("0")
    );
    assert_described_by_resolves(&second);
    root.remove();
}

#[wasm_bindgen_test]
async fn lifecycle_operations_settle_browser_state_and_replace_the_baseline() {
    let MountedTestApp {
        root,
        form_handle,
        submitted: _,
    } = mount_test_app(lifecycle_test_app).await;
    let quantity = control_with_binding(&form_handle, "/quantity");
    let input = input_with_binding(&root, "/quantity");
    let input_node: web_sys::Node = input
        .clone()
        .dyn_into()
        .expect("the quantity input should be a DOM node");
    let baseline: serde_json::Value =
        serde_json::from_str(r#"{"quantity":1e3}"#).expect("the baseline should parse");
    assert_eq!(input.value(), "1e+3");

    dispatch_input(&input, "0");
    dispatch_input(&input, "-");
    poll_dom(|| {
        let projection = form_handle.node(quantity).ok()??.read().ok()??;
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "quantity": 0 })
            && projection.edit_buffer.as_deref() == Some("-")
            && projection.parse_blocker == Some(ParseBlockerKind::InvalidInteger)
            && projection.dirty)
            .then_some(())
    })
    .await;
    let stale_revision = form_handle
        .reader()
        .read()
        .expect("form should be readable")
        .data_revision;
    form_handle
        .apply_external_findings(browser_blocking_batch(stale_revision))
        .expect("a current external finding should apply");
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Parse {
                    target,
                    kind: ParseBlockerKind::InvalidInteger,
                } if *target == quantity
            )) && blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Validation(_)
            )) && blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::External { source, finding }
                    if source == "server"
                        && finding.code() == "server-rejected"
                        && finding.instance_location().as_str() == "/quantity"
            ))
    ));
    let before_reset = form_handle
        .reader()
        .read()
        .expect("form should be readable");

    let reset = form_handle
        .reset()
        .expect("the form should reset without a borrow conflict");

    assert_eq!(reset.before_data_revision(), before_reset.data_revision);
    assert_eq!(reset.before_state_revision(), before_reset.state_revision);
    assert_ne!(reset.after_data_revision(), before_reset.data_revision);
    assert_ne!(reset.after_state_revision(), before_reset.state_revision);
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        baseline
    );
    assert_settled_lifecycle_state(&form_handle, quantity);
    poll_dom(|| (input.value() == "1e+3").then_some(())).await;
    assert!(
        form_handle
            .apply_external_findings(browser_blocking_batch(stale_revision))
            .is_err(),
        "reset data changes must make prior external batches stale"
    );
    let settled = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    let no_op = form_handle
        .reset()
        .expect("the no-op reset should not conflict with another handle borrow");
    assert!(no_op.is_empty());
    assert_eq!(
        form_handle
            .reader()
            .read()
            .expect("form should be readable")
            .data_revision,
        settled.data_revision
    );
    assert_eq!(
        form_handle
            .reader()
            .read()
            .expect("form should be readable")
            .state_revision,
        settled.state_revision
    );

    let before_invalid = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    let invalid_transition = form_handle
        .reinitialize(json!({ "quantity": 0 }))
        .expect("object data invalid against the data schema should remain repairable");
    assert_ne!(
        invalid_transition.after_data_revision(),
        before_invalid.data_revision
    );
    assert_ne!(
        invalid_transition.after_state_revision(),
        before_invalid.state_revision
    );
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({ "quantity": 0 })
    );
    poll_dom(|| (input.value() == "0").then_some(())).await;
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::Validation(_)
            ))
    ));
    poll_dom(|| {
        root.query_selector("[data-validation-finding='minimum']")
            .expect("the finding selector should be valid")
    })
    .await;

    let equal_revision = form_handle
        .reader()
        .read()
        .expect("form should be readable")
        .data_revision;
    form_handle
        .apply_external_findings(browser_blocking_batch(equal_revision))
        .expect("a finding for the invalid lifecycle should apply");
    input
        .focus()
        .expect("the equal invalid value should accept focus");
    dispatch_input(&input, "0e0");
    input.blur().expect("the equal invalid value should blur");
    poll_dom(|| {
        let projection = form_handle.node(quantity).ok()??.read().ok()??;
        (projection.edit_buffer.is_none() && projection.touched).then_some(())
    })
    .await;
    let before_equal = form_handle
        .reader()
        .read()
        .expect("form should be readable");

    let equal_transition = form_handle
        .reinitialize(json!({ "quantity": 0 }))
        .expect("equal data should still start a fresh lifecycle");

    assert_ne!(
        equal_transition.after_data_revision(),
        before_equal.data_revision
    );
    assert_ne!(
        equal_transition.after_state_revision(),
        before_equal.state_revision
    );
    assert_settled_lifecycle_state(&form_handle, quantity);
    poll_dom(|| {
        (input.value() == "0"
            && root
                .query_selector("[data-validation-finding='minimum']")
                .expect("the finding selector should be valid")
                .is_none())
        .then_some(())
    })
    .await;
    assert!(matches!(
        form_handle
            .prepare_submission()
            .expect("submission should not conflict with another handle borrow")
            .outcome(),
        SubmissionOutcome::Blocked(blockers)
            if !blockers.iter().any(|blocker| matches!(
                blocker,
                SubmissionBlocker::External { .. }
            ))
    ));
    assert!(
        form_handle
            .apply_external_findings(browser_blocking_batch(equal_revision))
            .is_err(),
        "equal reinitialization must invalidate work from the prior lifecycle"
    );

    form_handle
        .reinitialize(json!({ "quantity": 1000 }))
        .expect("valid replacement data should establish a new baseline");
    dispatch_input(&input, "2");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "quantity": 2 }))
        .then_some(())
    })
    .await;
    form_handle
        .reset()
        .expect("the form should reset without a borrow conflict");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "quantity": 1000 })
            && input.value() == "1000")
            .then_some(())
    })
    .await;

    let current_input_node: web_sys::Node = input_with_binding(&root, "/quantity")
        .dyn_into()
        .expect("the current quantity input should be a DOM node");
    assert!(input_node.is_same_node(Some(&current_input_node)));

    root.remove();
}

#[wasm_bindgen_test]
async fn external_visibility_parse_feedback_and_submission_focus_follow_core_policy() {
    let MountedTestApp {
        root,
        form_handle,
        submitted,
    } = mount_test_app(lifecycle_test_app).await;
    let input = input_with_binding(&root, "/quantity");
    let revision = form_handle
        .reader()
        .read()
        .expect("form should be readable")
        .data_revision;
    form_handle
        .apply_external_findings(browser_blocking_batch(revision))
        .expect("the current external finding should apply");
    form_handle
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            revision,
            [ExternalFinding::blocking(
                "server-retry-required",
                JsonPointer::parse("/quantity").expect("the quantity pointer should be valid"),
                json!({ "attempt": 2 }),
            )],
        ))
        .expect("the same source should replace its current browser finding");
    next_microtask().await;
    assert!(
        root.query_selector("[data-external-finding='server-retry-required']")
            .expect("the external-finding selector should be valid")
            .is_none(),
        "default visibility should hide an untouched external finding"
    );

    dispatch_input(&input, "-");
    let parse = poll_dom(|| {
        root.query_selector("[data-parse-blocker='invalid-integer']")
            .expect("the parse-blocker selector should be valid")
    })
    .await;
    let parse_id = parse.id();
    assert!(submitted.borrow().is_none());

    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    dispatch_submit(&form);
    let external = poll_dom(|| {
        root.query_selector("[data-external-finding='server-retry-required']")
            .expect("the external-finding selector should be valid")
    })
    .await;
    assert_eq!(
        external.get_attribute("data-blocking").as_deref(),
        Some("true")
    );
    assert_eq!(
        root.query_selector("[data-parse-blocker='invalid-integer']")
            .expect("the parse-blocker selector should be valid")
            .expect("the parse blocker should remain visible")
            .id(),
        parse_id,
        "revealing an earlier summary finding must not renumber a stable finding ID"
    );
    assert!(submitted.borrow().is_none());
    let summary = root
        .query_selector("[data-finding-summary]")
        .expect("the summary selector should be valid")
        .expect("the form should expose a finding summary");
    let focused = web_sys::window()
        .expect("the browser should have a window")
        .document()
        .expect("the browser should have a document")
        .active_element()
        .expect("blocked submission should focus the summary");
    assert_eq!(focused.id(), summary.id());
    accessibility_checkpoint(
        "external-parse-blocked",
        "external_visibility_parse_feedback_and_submission_focus_follow_core_policy",
        &root,
    )
    .await;

    dispatch_input(&input, "2");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "quantity": 2 })
            && root
                .query_selector("[data-external-finding='server-retry-required']")
                .expect("the external-finding selector should be valid")
                .is_none()
            && root
                .query_selector("[data-parse-blocker]")
                .expect("the parse-blocker selector should be valid")
                .is_none())
        .then_some(())
    })
    .await;
    let before_stale = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert!(
        form_handle
            .apply_external_findings(browser_blocking_batch(revision))
            .is_err(),
        "the data change should make the old browser batch stale"
    );
    let after_stale = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(after_stale.data_revision, before_stale.data_revision);
    assert_eq!(after_stale.state_revision, before_stale.state_revision);
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &json!({ "quantity": 2 }));

    root.remove();
}

#[wasm_bindgen_test]
async fn custom_presenters_receive_one_deterministic_collection_per_target() {
    let MountedTestApp {
        root,
        form_handle,
        submitted: _,
    } = mount_test_app(presenter_collection_test_app).await;
    let revision = form_handle
        .reader()
        .read()
        .expect("form should be readable")
        .data_revision;
    form_handle
        .apply_external_findings(ExternalFindingBatch::new(
            "server",
            revision,
            [ExternalFinding::blocking(
                "quantity-rejected",
                JsonPointer::parse("/quantity").expect("the quantity pointer should be valid"),
                json!({ "minimum": 2, "received": 1 }),
            )],
        ))
        .expect("the current external finding should apply");
    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .expect("the schema form should render")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    let submit = form
        .query_selector("button[type='submit']")
        .unwrap()
        .expect("the schema form should render a submit button");
    assert_eq!(submit.text_content().as_deref(), Some("Submit"));
    dispatch_submit(&form);

    poll_dom(|| {
        let control = input_with_binding(&root, "/quantity")
            .parent_element()
            .expect("the quantity input should have a control container");
        let collections = control
            .query_selector_all("[data-descriptor-collection='local']")
            .ok()?;
        let collection: web_sys::Element = collections.get(0)?.dyn_into().ok()?;
        (collections.length() == 1
            && collection.get_attribute("data-descriptor-count").as_deref() == Some("2"))
        .then_some(())
    })
    .await;
    let control = input_with_binding(&root, "/quantity")
        .parent_element()
        .expect("the quantity input should have a control container");
    let local = control
        .query_selector("[data-descriptor-collection='local']")
        .unwrap()
        .expect("the control should have one local finding collection");
    assert_eq!(
        local.get_attribute("data-descriptor-count").as_deref(),
        Some("2")
    );
    let descriptors = local
        .query_selector_all("[data-descriptor-code]")
        .expect("the descriptor selector should be valid");
    assert_eq!(descriptors.length(), 2);
    let first_descriptor: web_sys::Element = descriptors.get(0).unwrap().dyn_into().unwrap();
    let second_descriptor: web_sys::Element = descriptors.get(1).unwrap().dyn_into().unwrap();
    assert_eq!(
        first_descriptor
            .get_attribute("data-descriptor-code")
            .as_deref(),
        Some("minimum")
    );
    assert_eq!(
        second_descriptor
            .get_attribute("data-descriptor-code")
            .as_deref(),
        Some("quantity-rejected")
    );
    assert_eq!(
        second_descriptor
            .get_attribute("data-descriptor-parameters")
            .as_deref(),
        Some(r#"{"minimum":2,"received":1}"#)
    );

    let summary = root
        .query_selector("[data-finding-summary] [data-descriptor-collection='summary']")
        .unwrap()
        .expect("the summary presenter should receive the aggregate collection");
    assert_eq!(
        summary.get_attribute("data-descriptor-count").as_deref(),
        Some("2")
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn locale_and_presenter_changes_update_only_reactive_plain_text_presentation() {
    let MountedTestApp {
        root,
        form_handle,
        submitted: _,
    } = mount_test_app(reactive_presentation_test_app).await;
    let input = input_with_binding(&root, "/quantity");
    let input_node: web_sys::Node = input.clone().dyn_into().unwrap();
    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .expect("the schema form should render")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    let submit = form
        .query_selector("button[type='submit']")
        .unwrap()
        .expect("the schema form should render a submit button");
    assert_eq!(submit.text_content().as_deref(), Some("Submit"));
    dispatch_submit(&form);
    let local_finding = poll_dom(|| {
        root.query_selector("[data-reactive-presenter='local'] [data-reactive-finding='minimum']")
            .ok()?
    })
    .await;
    assert_eq!(
        root.query_selector_all("[data-presentation-renderer] [data-reactive-presenter='local']")
            .unwrap()
            .length(),
        1,
        "the custom renderer places the local collection through present_findings()"
    );
    assert_eq!(
        root.query_selector_all(
            "[data-reactive-presenter='local'] [data-reactive-finding='minimum']"
        )
        .unwrap()
        .length(),
        1,
        "the adapter must not render a second local collection after the custom control"
    );
    let described_by = assert_described_by_resolves(&input_with_binding(&root, "/quantity"));
    assert!(
        described_by.iter().any(|id| id == &local_finding.id()),
        "the custom control must be described by the presented finding"
    );
    let summary_finding = root
        .query_selector("[data-reactive-presenter='summary'] [data-reactive-finding='minimum']")
        .unwrap()
        .expect("the summary presenter should receive the minimum finding");
    let local_id = local_finding.id();
    let summary_id = summary_finding.id();
    root.query_selector("button[data-append-item]")
        .unwrap()
        .expect("the reactive array should expose its add action")
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    let array_status = poll_dom(|| {
        let status = root.query_selector("[data-array-status]").ok()??;
        (status.text_content().as_deref() == Some("Tags item added at position 2."))
            .then_some(status)
    })
    .await;
    let array_status_node: web_sys::Node = array_status.clone().into();
    let locale_button: web_sys::HtmlElement = root
        .query_selector("[data-change-locale]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    locale_button.focus().unwrap();
    next_microtask().await;
    next_microtask().await;
    let before = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    let before_data = form_handle
        .reader()
        .form_data()
        .expect("form should be readable");

    locale_button.click();
    let localized_intro = poll_dom(|| {
        let text = root.query_selector("[data-schemaform-text]").ok()??;
        (text.text_content().as_deref() == Some("Localized <strong>intro</strong>."))
            .then_some(text)
    })
    .await;
    assert!(
        localized_intro.query_selector("strong").unwrap().is_none(),
        "localized presentation must remain escaped plain text"
    );
    let current_input = input_with_binding(&root, "/quantity");
    let current_input_node: web_sys::Node = current_input.clone().dyn_into().unwrap();
    assert!(input_node.is_same_node(Some(&current_input_node)));
    let label = root
        .query_selector(&format!("label[for='{}']", current_input.id()))
        .unwrap()
        .expect("the generated control should remain labeled");
    assert_eq!(label.text_content().as_deref(), Some("Localized Quantity"));
    assert_eq!(submit.text_content().as_deref(), Some("Localized Submit"));
    let localized_array_status = poll_dom(|| {
        let status = root.query_selector("[data-array-status]").ok()??;
        (status.text_content().as_deref() == Some("Localized item added at position 2."))
            .then_some(status)
    })
    .await;
    let localized_array_status_node: web_sys::Node = localized_array_status.into();
    assert!(array_status_node.is_same_node(Some(&localized_array_status_node)));
    let fixed_object = root
        .query_selector("[data-schemaform-fixed-object]")
        .unwrap()
        .expect("the generated fixed object should render");
    assert_eq!(
        fixed_object
            .query_selector("legend")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Localized Settings")
    );
    assert_eq!(
        fixed_object
            .query_selector(".schemaform-help")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Localized Account settings.")
    );

    let localized_local = root
        .query_selector("[data-reactive-presenter='local'] [data-reactive-finding='minimum']")
        .unwrap()
        .expect("the local finding should remain rendered");
    assert_eq!(localized_local.id(), local_id);
    assert_eq!(
        localized_local.text_content().as_deref(),
        Some("Localized minimum <strong>2</strong>.")
    );
    assert!(localized_local.query_selector("strong").unwrap().is_none());
    assert!(
        localized_local.query_selector("button").unwrap().is_some(),
        "the local presenter should receive its prepared target focus action"
    );
    let localized_summary = root
        .query_selector("[data-reactive-presenter='summary'] [data-reactive-finding='minimum']")
        .unwrap()
        .expect("the summary finding should remain rendered");
    assert_eq!(localized_summary.id(), summary_id);
    assert_eq!(
        localized_summary
            .get_attribute("data-reactive-parameters")
            .as_deref(),
        Some(r#"{"limit":2}"#)
    );
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        before_data
    );
    let after_locale = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(after_locale.data_revision, before.data_revision);
    assert_eq!(after_locale.state_revision, before.state_revision);
    let renderer_calls = root
        .query_selector("[data-presentation-renderer]")
        .unwrap()
        .unwrap()
        .get_attribute("data-renderer-calls")
        .expect("the custom renderer should expose its call count");

    let summary_action: web_sys::HtmlElement = localized_summary
        .query_selector("button")
        .unwrap()
        .expect("the summary presenter should receive a prepared focus action")
        .dyn_into()
        .unwrap();
    summary_action.click();
    assert_eq!(
        web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element()
            .unwrap()
            .id(),
        current_input.id()
    );

    let local_button: web_sys::HtmlElement = root
        .query_selector("[data-change-local-presenter]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    local_button.click();
    poll_dom(|| {
        let local = root
            .query_selector("[data-reactive-presenter='local']")
            .ok()??;
        (local.get_attribute("data-presenter-mode").as_deref() == Some("alternate")).then_some(())
    })
    .await;
    let summary = root
        .query_selector("[data-reactive-presenter='summary']")
        .unwrap()
        .unwrap();
    assert_eq!(
        summary.get_attribute("data-presenter-mode").as_deref(),
        Some("default")
    );
    assert_eq!(
        root.query_selector("[data-presentation-renderer]")
            .unwrap()
            .unwrap()
            .get_attribute("data-renderer-calls")
            .as_deref(),
        Some(renderer_calls.as_str()),
        "replacing the local presenter must not rerender the control"
    );

    let summary_button: web_sys::HtmlElement = root
        .query_selector("[data-change-summary-presenter]")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    summary_button.click();
    poll_dom(|| {
        let summary = root
            .query_selector("[data-reactive-presenter='summary']")
            .ok()??;
        (summary.get_attribute("data-presenter-mode").as_deref() == Some("alternate")).then_some(())
    })
    .await;
    let final_projection = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        before_data
    );
    assert_eq!(final_projection.data_revision, before.data_revision);
    assert_eq!(final_projection.state_revision, before.state_revision);
    assert_eq!(
        root.query_selector("[data-presentation-renderer]")
            .unwrap()
            .unwrap()
            .get_attribute("data-renderer-calls")
            .as_deref(),
        Some(renderer_calls.as_str()),
        "replacing the summary presenter must not rerender the control"
    );
    let final_input_node: web_sys::Node =
        input_with_binding(&root, "/quantity").dyn_into().unwrap();
    assert!(input_node.is_same_node(Some(&final_input_node)));

    root.remove();
}

#[wasm_bindgen_test]
async fn ime_composition_stays_local_across_presentation_updates_and_commits_on_end() {
    let MountedTestApp {
        root, form_handle, ..
    } = mount_test_app(ime_test_app).await;
    let input = input_with_binding(&root, "/quantity");
    let input_node: web_sys::Node = input.clone().dyn_into().unwrap();
    let identity = control_with_binding(&form_handle, "/quantity");
    let before = form_handle
        .reader()
        .read()
        .expect("form should be readable");

    dispatch_composition(&input, "compositionstart", "");
    dispatch_input(&input, "-");
    next_microtask().await;
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({ "quantity": 1 })
    );
    let composing = form_handle
        .node(identity)
        .unwrap()
        .unwrap()
        .read()
        .unwrap()
        .unwrap();
    assert_eq!(composing.parse_blocker, None);
    assert_eq!(composing.edit_buffer, None);
    assert_eq!(input.value(), "-");

    root.query_selector("[data-ime-change-locale]")
        .unwrap()
        .unwrap()
        .dyn_into::<web_sys::HtmlElement>()
        .unwrap()
        .click();
    poll_dom(|| {
        let label = root
            .query_selector(&format!("label[for='{}']", input.id()))
            .ok()??;
        (label.text_content().as_deref() == Some("Localized quantity")).then_some(())
    })
    .await;
    let current = input_with_binding(&root, "/quantity");
    let current_node: web_sys::Node = current.clone().dyn_into().unwrap();
    assert!(input_node.is_same_node(Some(&current_node)));
    assert_eq!(current.value(), "-");
    let after_locale = form_handle
        .reader()
        .read()
        .expect("form should be readable");
    assert_eq!(after_locale.data_revision, before.data_revision);
    assert_eq!(after_locale.state_revision, before.state_revision);

    dispatch_input(&current, "3");
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable"),
        json!({ "quantity": 1 })
    );
    dispatch_composition(&current, "compositionend", "3");
    poll_dom(|| {
        (form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
            == json!({ "quantity": 3 }))
        .then_some(())
    })
    .await;
    let committed = form_handle
        .node(identity)
        .unwrap()
        .unwrap()
        .read()
        .unwrap()
        .unwrap();
    assert_eq!(committed.parse_blocker, None);
    assert_eq!(committed.edit_buffer.as_deref(), Some("3"));
    assert_ne!(
        form_handle
            .reader()
            .read()
            .expect("form should be readable")
            .data_revision,
        before.data_revision
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn oversized_input_reports_typed_error_and_resynchronizes_before_submission() {
    let (
        MountedTestApp {
            root,
            form_handle,
            submitted,
        },
        errors,
    ) = mount_test_app_with_errors(ime_test_app).await;
    let input = input_with_binding(&root, "/quantity");
    let oversized = "9".repeat(512 * 1024 + 1);

    dispatch_paste_input(&input, &oversized);

    assert_eq!(input.value(), "1");
    assert_eq!(
        form_handle
            .reader()
            .form_data()
            .expect("form should remain readable after rejected input"),
        json!({ "quantity": 1 })
    );
    assert!(matches!(
        errors.borrow().as_slice(),
        [HandleError::UserOperation(
            schemaform::form::UserOperationError::ResourceLimit(limit)
        )] if limit.dimension() == "edit_buffer_bytes"
            && limit.maximum() == 512 * 1024
            && limit.observed() == oversized.len()
    ));

    let form: HtmlFormElement = root
        .query_selector("form")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &json!({ "quantity": 1 }));
    assert!(matches!(
        form_handle.reinitialize(json!(null)),
        Err(HandleError::Reinitialize(
            schemaform::form::ReinitializeError::InvalidFormData
        ))
    ));
    let stale_revision = form_handle.reader().read().unwrap().data_revision;
    form_handle
        .reinitialize(json!({ "quantity": 1 }))
        .expect("equal reinitialization should advance the lifecycle");
    assert!(matches!(
        form_handle.apply_external_findings(browser_blocking_batch(stale_revision)),
        Err(HandleError::ExternalFindings(
            schemaform::form::ExternalFindingError::StaleRevision { .. }
        ))
    ));

    root.remove();
}

#[wasm_bindgen_test]
async fn lifecycle_changes_discard_composition_without_stale_overwrite() {
    let (
        MountedTestApp {
            root, form_handle, ..
        },
        errors,
    ) = mount_test_app_with_errors(ime_test_app).await;
    let input = input_with_binding(&root, "/quantity");

    dispatch_composition(&input, "compositionstart", "");
    dispatch_input(&input, "-");
    form_handle
        .reinitialize(json!({ "quantity": 7 }))
        .expect("reinitialization should establish a new lifecycle");
    poll_dom(|| (input.value() == "7").then_some(())).await;
    dispatch_composition(&input, "compositionend", "-");
    next_microtask().await;
    assert_eq!(
        form_handle.reader().form_data().unwrap(),
        json!({ "quantity": 7 })
    );
    assert_eq!(input.value(), "7");

    dispatch_composition(&input, "compositionstart", "");
    dispatch_input(&input, "8");
    assert_eq!(input.value(), "8");
    form_handle
        .reset()
        .expect("a no-op reset should still start a new adapter lifecycle");
    poll_dom(|| (input.value() == "7").then_some(())).await;
    dispatch_composition(&input, "compositionend", "8");
    next_microtask().await;
    assert_eq!(
        form_handle.reader().form_data().unwrap(),
        json!({ "quantity": 7 })
    );
    assert_eq!(input.value(), "7");
    assert!(errors.borrow().is_empty());

    root.remove();
}

#[wasm_bindgen_test]
async fn every_builtin_user_operation_failure_reaches_schema_form_on_error() {
    let (
        MountedTestApp {
            root, form_handle, ..
        },
        errors,
    ) = mount_test_app_with_errors(operation_error_test_app).await;
    let text = input_with_binding(&root, "/text");
    let checkbox = input_with_binding(&root, "/enabled");
    let choice = select_with_binding(&root, "/choice");
    let canonical_choice = choice.value();
    text.focus().unwrap();

    form_handle
        .try_transact(|_| {
            dispatch_input(&text, "rejected");
            text.blur().unwrap();
            root.query_selector("[data-set-value]")
                .unwrap()
                .unwrap()
                .dyn_into::<web_sys::HtmlElement>()
                .unwrap()
                .click();
            dispatch_checkbox_input(&checkbox, true);
            dispatch_select_alternative(&choice);
            for selector in [
                "[data-insert-item-before]",
                "[data-move-item-up]",
                "[data-move-item-down]",
                "[data-remove-item]",
                "[data-append-item]",
            ] {
                root.query_selector(selector)
                    .unwrap()
                    .unwrap()
                    .dyn_into::<web_sys::HtmlElement>()
                    .unwrap()
                    .click();
            }
            Ok::<_, ()>(())
        })
        .expect("the outer transaction should complete without mutation");

    assert_eq!(text.value(), "canonical");
    assert!(!checkbox.checked());
    assert_eq!(choice.value(), canonical_choice);
    assert_eq!(
        form_handle.reader().form_data().unwrap(),
        json!({ "text": "canonical", "enabled": false, "choice": "a", "items": ["one", "two"] })
    );
    assert_eq!(errors.borrow().len(), 10);
    assert!(
        errors
            .borrow()
            .iter()
            .all(|error| matches!(error, HandleError::BorrowConflict))
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn custom_renderer_reported_failures_reach_schema_form_on_error() {
    let (
        MountedTestApp {
            root, form_handle, ..
        },
        errors,
    ) = mount_test_app_with_errors(custom_renderer_operation_error_test_app).await;
    let text = input_with_binding(&root, "/text");
    let optional = input_with_binding(&root, "/optional");
    assert!(text.has_attribute("data-affordance-widget"));
    let set_optional = affordance_button(&root, &optional, "Set");

    form_handle
        .try_transact(|_| {
            // The host holds the form borrow: the renderer's `report()` and the affordance's
            // internal reporting must both route the conflict to `on_error`.
            dispatch_input(&text, "rejected");
            set_optional.click();
            Ok::<_, ()>(())
        })
        .expect("the outer transaction should complete without mutation");

    assert_eq!(
        form_handle.reader().form_data().unwrap(),
        json!({ "text": "canonical" })
    );
    assert_eq!(
        *errors.borrow(),
        vec![HandleError::BorrowConflict, HandleError::BorrowConflict]
    );

    root.remove();
}

#[wasm_bindgen_test]
async fn custom_shell_renderer_keeps_submission_and_summary_focus_behaviour() {
    let (
        MountedTestApp {
            root,
            form_handle,
            submitted,
        },
        errors,
    ) = mount_test_app_with_errors(custom_shell_test_app).await;
    let form: HtmlFormElement = root
        .query_selector("form")
        .expect("the form selector should be valid")
        .expect("the schema form should render a form element")
        .dyn_into()
        .expect("the schema form should use semantic form HTML");
    let form_id = form.id();

    // The adapter keeps the form element and its submission contract.
    assert!(form.no_validate());
    assert_eq!(form.get_attribute("tabindex").as_deref(), Some("-1"));
    assert!(form.has_attribute("data-schemaform"));

    // The shell placed the adapter-owned summary wrapper and the body where it wanted, in its
    // own order, and did not render a `type="submit"` button.
    let summary = form
        .query_selector("aside[data-test-shell='summary'] > [data-finding-summary]")
        .expect("the summary selector should be valid")
        .expect("the shell should place the adapter-owned summary wrapper inside its aside");
    assert_eq!(summary.id(), format!("{form_id}-summary"));
    assert!(
        form.query_selector("section[data-test-shell='body'] input[name='/name']")
            .expect("the body selector should be valid")
            .is_some(),
        "the shell should place the body in its section"
    );
    assert!(
        form.query_selector("button[type='submit']")
            .expect("the submit selector should be valid")
            .is_none(),
        "the adapter must not add its own submit button under a custom shell"
    );
    let submit: web_sys::HtmlElement = form
        .query_selector("footer[data-test-shell='footer'] > button[data-test-shell-submit]")
        .expect("the shell submit selector should be valid")
        .expect("the shell should render the submit affordance")
        .dyn_into()
        .expect("the shell submit should be a button");
    assert_eq!(submit.id(), format!("{form_id}-submit"));
    assert_eq!(submit.text_content().as_deref(), Some("Submit"));
    assert_eq!(
        submit.get_attribute("data-affordance").as_deref(),
        Some("Submit")
    );
    assert_eq!(
        form.child_element_count(),
        3,
        "the form contains exactly the shell's output"
    );

    // A blocked submit through the affordance yields no snapshot and focuses the summary.
    let input = input_with_binding(&root, "/name");
    dispatch_input(&input, "A");
    poll_dom(|| (form_handle.reader().form_data().ok()? == json!({ "name": "A" })).then_some(()))
        .await;
    submit.click();
    next_microtask().await;
    assert!(submitted.borrow().is_none());
    wait_for_summary_focus(&root).await;
    poll_dom(|| {
        summary
            .query_selector("[data-finding='minLength']")
            .expect("the finding selector should be valid")
    })
    .await;

    // A ready submit through the affordance yields an immutable snapshot.
    dispatch_input(&input, "Grace");
    let grace_data = json!({ "name": "Grace" });
    poll_dom(|| (form_handle.reader().form_data().ok()? == grace_data).then_some(())).await;
    submit.click();
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &grace_data);

    // The form element's own submit event (implicit submission) still runs the adapter's
    // handler under a custom shell.
    *submitted.borrow_mut() = None;
    dispatch_submit(&form);
    let snapshot = poll_dom(|| submitted.borrow().clone()).await;
    assert_eq!(snapshot.form_data(), &grace_data);

    dispatch_input(&input, "Lin");
    poll_dom(|| (form_handle.reader().form_data().ok()? == json!({ "name": "Lin" })).then_some(()))
        .await;
    assert_eq!(snapshot.form_data(), &grace_data);
    assert!(errors.borrow().is_empty());

    root.remove();
}

/// Finds the element with `id`, failing with the id in the message.
fn element_by_id(id: &str) -> web_sys::HtmlElement {
    web_sys::window()
        .expect("the browser test should run in a window")
        .document()
        .expect("the browser test should have a document")
        .get_element_by_id(id)
        .unwrap_or_else(|| panic!("the element #{id} should exist"))
        .dyn_into()
        .expect("the element should be an HTML element")
}

fn maybe_element_by_id(id: &str) -> Option<web_sys::Element> {
    web_sys::window()?.document()?.get_element_by_id(id)
}

async fn wait_for_focus_on(id: &str, operation: &str) {
    for _ in 0..200 {
        let focused = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element());
        if focused.as_ref().is_some_and(|focused| focused.id() == id) {
            return;
        }
        next_microtask().await;
    }
    let actual = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.active_element())
        .map(|focused| focused.id())
        .unwrap_or_default();
    panic!("{operation} focused #{actual}, expected #{id}");
}

async fn wait_for_announcement(status: &web_sys::Element, expected: &str) {
    let expected = expected.to_owned();
    poll_dom(|| (status.text_content()? == expected).then_some(())).await;
}

#[wasm_bindgen_test]
async fn custom_collection_renderer_keeps_identity_focus_announcements_and_presence_repair() {
    let (
        MountedTestApp {
            root, form_handle, ..
        },
        errors,
    ) = mount_test_app_with_errors(custom_collection_test_app).await;
    accessibility_checkpoint(
        "array-custom-collection",
        "custom_collection_renderer_keeps_identity_focus_announcements_and_presence_repair",
        &root,
    )
    .await;
    let collection = poll_dom(|| {
        root.query_selector("section[data-test-collection]")
            .expect("the collection selector should be valid")
    })
    .await;
    let element_id = collection.id();
    assert!(
        root.query_selector("fieldset[data-schemaform-array]")
            .expect("the built-in selector should be valid")
            .is_none(),
        "the built-in collection chrome must not render under a custom collection renderer"
    );
    assert!(
        root.query_selector(
            "[data-append-item], [data-insert-item-before], [data-move-item-up], [data-move-item-down], [data-remove-item], [data-materialize], [data-remove-value]"
        )
        .expect("the marker selector should be valid")
        .is_none(),
        "the test renderer emits no built-in markers, so every button below is found by affordance id"
    );
    assert_eq!(
        collection.get_attribute("data-test-count").as_deref(),
        Some("2")
    );
    assert_eq!(
        collection
            .query_selector("h2")
            .unwrap()
            .unwrap()
            .text_content()
            .as_deref(),
        Some("Tags")
    );
    assert_described_by_resolves(&collection);

    // The adapter owns the keyed row wrapper; the renderer's item output sits inside it and
    // uses `row_id` only as a prefix.
    let first = input_with_binding(&root, "/tags/0");
    let second = input_with_binding(&root, "/tags/1");
    let first_id = first.id();
    let second_id = second.id();
    let rows = collection.query_selector_all("[data-array-item]").unwrap();
    assert_eq!(rows.length(), 2);
    let first_row: web_sys::Element = rows.get(0).unwrap().dyn_into().unwrap();
    assert_eq!(first_row.id(), format!("{first_id}-row"));
    let first_item = first_row
        .first_element_child()
        .expect("the row wrapper contains the renderer's item");
    assert!(first_item.has_attribute("data-test-item"));
    assert_eq!(
        first_item.get_attribute("aria-labelledby").as_deref(),
        Some(format!("{first_id}-row-title").as_str())
    );
    assert_eq!(
        element_by_id(&format!("{first_id}-row-title"))
            .text_content()
            .as_deref(),
        Some("Tags item 1/2")
    );
    // The renderer put its buttons before the children.
    let first_actions = first_item.query_selector_all("header > button").unwrap();
    assert!(first_actions.length() >= 3);
    assert!(
        first_item
            .query_selector("header ~ * input")
            .unwrap()
            .is_some(),
        "the item's input follows the renderer's header of action buttons"
    );
    assert!(maybe_element_by_id(&format!("{first_id}-move-up")).is_none());
    assert!(maybe_element_by_id(&format!("{second_id}-move-down")).is_none());

    // The adapter-owned live region is present even though the renderer wrapped it.
    let status = collection
        .query_selector(".visually-hidden > [data-array-status]")
        .unwrap()
        .expect("the renderer placed the adapter's live region inside its wrapper");
    assert_eq!(status.get_attribute("role").as_deref(), Some("status"));
    assert_eq!(status.get_attribute("aria-live").as_deref(), Some("polite"));
    assert_eq!(status.get_attribute("aria-atomic").as_deref(), Some("true"));

    // Insert before the second item: the new input receives focus even though the renderer's
    // buttons precede it in DOM order.
    let insert = element_by_id(&format!("{second_id}-insert-before"));
    assert_eq!(
        insert.text_content().as_deref(),
        Some("Insert Tags item before")
    );
    assert_eq!(
        insert.get_attribute("aria-label").as_deref(),
        Some("Insert Tags item before position 2")
    );
    assert_eq!(
        insert.get_attribute("data-test-affordance").as_deref(),
        Some("InsertBefore")
    );
    insert.focus().unwrap();
    insert.click();
    poll_dom(|| {
        (form_handle.reader().form_data().ok()? == json!({ "tags": ["same", "valid", "same"] }))
            .then_some(())
    })
    .await;
    let inserted = poll_dom(|| {
        let inputs = collection.query_selector_all("input").ok()?;
        (inputs.length() == 3).then(|| input_with_binding(&root, "/tags/1"))
    })
    .await;
    let inserted_id = inserted.id();
    assert_ne!(inserted_id, first_id);
    assert_ne!(inserted_id, second_id);
    wait_for_focus_on(&inserted_id, "insert before").await;
    wait_for_announcement(&status, "Tags item inserted at position 2.").await;
    assert!(first.is_same_node(Some(&input_with_binding(&root, "/tags/0"))));
    assert!(second.is_same_node(Some(&input_with_binding(&root, "/tags/2"))));
    assert_eq!(
        collection.get_attribute("data-test-count").as_deref(),
        Some("3")
    );
    assert_eq!(
        element_by_id(&format!("{second_id}-row-title"))
            .text_content()
            .as_deref(),
        Some("Tags item 3/3")
    );

    // Move the first item down: it keeps its DOM node and focus lands on its move-down button.
    let move_down = element_by_id(&format!("{first_id}-move-down"));
    assert_eq!(
        move_down.get_attribute("aria-label").as_deref(),
        Some("Move Tags item at position 1 down")
    );
    move_down.focus().unwrap();
    move_down.click();
    poll_dom(|| {
        (form_handle.reader().form_data().ok()? == json!({ "tags": ["valid", "same", "same"] }))
            .then_some(())
    })
    .await;
    wait_for_focus_on(&format!("{first_id}-move-down"), "move down").await;
    wait_for_announcement(&status, "Tags item moved down to position 2.").await;
    assert!(first.is_same_node(Some(&input_with_binding(&root, "/tags/1"))));
    assert!(inserted.is_same_node(Some(&input_with_binding(&root, "/tags/0"))));

    // Move it back up: no move-up button remains for the first item, so focus falls back to
    // its move-down button.
    let move_up = element_by_id(&format!("{first_id}-move-up"));
    move_up.focus().unwrap();
    move_up.click();
    poll_dom(|| {
        (form_handle.reader().form_data().ok()? == json!({ "tags": ["same", "valid", "same"] }))
            .then_some(())
    })
    .await;
    wait_for_focus_on(&format!("{first_id}-move-down"), "move up").await;
    wait_for_announcement(&status, "Tags item moved up to position 1.").await;
    assert!(maybe_element_by_id(&format!("{first_id}-move-up")).is_none());

    // Remove the inserted item: focus moves to the next item's input.
    let remove = element_by_id(&format!("{inserted_id}-remove"));
    assert_eq!(
        remove.get_attribute("aria-label").as_deref(),
        Some("Remove Tags item at position 2")
    );
    remove.focus().unwrap();
    remove.click();
    poll_dom(|| {
        (form_handle.reader().form_data().ok()? == json!({ "tags": ["same", "same"] }))
            .then_some(())
    })
    .await;
    wait_for_focus_on(&second_id, "remove").await;
    wait_for_announcement(&status, "Tags item removed from position 2.").await;
    assert!(second.is_same_node(Some(&input_with_binding(&root, "/tags/1"))));
    assert!(maybe_element_by_id(&inserted_id).is_none());

    // Append: the new input receives focus and the append affordance disappears at maxItems.
    let append = element_by_id(&format!("{element_id}-append"));
    assert_eq!(append.text_content().as_deref(), Some("Add Tags item"));
    assert_eq!(
        append.get_attribute("data-test-affordance").as_deref(),
        Some("Append")
    );
    append.focus().unwrap();
    append.click();
    poll_dom(|| {
        (form_handle.reader().form_data().ok()? == json!({ "tags": ["same", "same", "valid"] }))
            .then_some(())
    })
    .await;
    let appended = poll_dom(|| {
        let inputs = collection.query_selector_all("input").ok()?;
        (inputs.length() == 3).then(|| input_with_binding(&root, "/tags/2"))
    })
    .await;
    wait_for_focus_on(&appended.id(), "append").await;
    wait_for_announcement(&status, "Tags item added at position 3.").await;
    poll_dom(|| {
        maybe_element_by_id(&format!("{element_id}-append"))
            .is_none()
            .then_some(())
    })
    .await;
    assert!(first.is_same_node(Some(&input_with_binding(&root, "/tags/0"))));
    assert!(second.is_same_node(Some(&input_with_binding(&root, "/tags/1"))));

    // Presence repair through the presentation's affordances: remove the optional array (the
    // renderer shows its empty state), materialize it, then replace incompatible data. Each
    // operation focuses the renderer's root and announces.
    let remove_value = element_by_id(&format!("{element_id}-remove-value"));
    assert_eq!(remove_value.text_content().as_deref(), Some("Remove Tags"));
    remove_value.click();
    poll_dom(|| (form_handle.reader().form_data().ok()? == json!({})).then_some(())).await;
    wait_for_focus_on(&element_id, "remove value").await;
    wait_for_announcement(&status, "Tags removed.").await;
    poll_dom(|| {
        collection
            .query_selector("[data-test-empty]")
            .expect("the empty-state selector should be valid")
    })
    .await;
    assert_eq!(
        collection.get_attribute("data-test-count").as_deref(),
        Some("0")
    );
    assert!(maybe_element_by_id(&format!("{element_id}-remove-value")).is_none());

    let materialize = element_by_id(&format!("{element_id}-materialize"));
    assert_eq!(materialize.text_content().as_deref(), Some("Add Tags"));
    materialize.click();
    poll_dom(|| {
        (form_handle.reader().form_data().ok()? == json!({ "tags": ["seed"] })).then_some(())
    })
    .await;
    wait_for_focus_on(&element_id, "materialize").await;
    wait_for_announcement(&status, "Tags added.").await;
    poll_dom(|| {
        collection
            .query_selector("[data-test-empty]")
            .ok()?
            .is_none()
            .then_some(())
    })
    .await;

    form_handle
        .try_transact(|draft| {
            draft.set(&JsonPointer::parse("/tags").unwrap(), json!("legacy"));
            Ok::<_, ()>(())
        })
        .expect("the host should install incompatible array data");
    let incompatible = poll_dom(|| {
        collection
            .query_selector("[data-test-incompatible]")
            .expect("the incompatible-value selector should be valid")
    })
    .await;
    assert_eq!(incompatible.text_content().as_deref(), Some("\"legacy\""));
    let replace = element_by_id(&format!("{element_id}-replace-value"));
    assert_eq!(replace.text_content().as_deref(), Some("Replace Tags"));
    replace.click();
    poll_dom(|| {
        (form_handle.reader().form_data().ok()? == json!({ "tags": ["seed"] })).then_some(())
    })
    .await;
    wait_for_focus_on(&element_id, "replace").await;
    wait_for_announcement(&status, "Tags replaced.").await;

    // Reset restores the baseline items with their original DOM ids: keying by instance identity
    // is the adapter's, not the renderer's.
    form_handle
        .reset()
        .expect("the collection form should reset without a borrow conflict");
    poll_dom(|| {
        if form_handle.reader().form_data().ok()? != json!({ "tags": ["same", "same"] }) {
            return None;
        }
        let first = maybe_input_with_binding(&root, "/tags/0")?;
        let second = maybe_input_with_binding(&root, "/tags/1")?;
        (first.id() == first_id && second.id() == second_id).then_some(())
    })
    .await;
    assert!(errors.borrow().is_empty());

    root.remove();
}

/// A collection renderer that only counts how often the adapter calls it. Its output is the
/// minimum the contract requires: the announcement, the item hosts, and the append affordance.
#[derive(Clone)]
struct CountingCollection {
    collection_calls: Rc<RefCell<usize>>,
    item_calls: Rc<RefCell<usize>>,
}

impl CollectionRenderer for CountingCollection {
    fn collection(&self, context: CollectionContext) -> Element {
        *self.collection_calls.borrow_mut() += 1;
        rsx! {
            section {
                id: context.presentation.element_id.clone(),
                "data-counting-collection": "",
                "data-test-count": "{context.count}",
                {context.announcement}
                {context.items}
                if let Some(append) = context.append {
                    {test_affordance_button(append)}
                }
            }
        }
    }

    fn collection_item(&self, context: CollectionItemContext) -> Element {
        *self.item_calls.borrow_mut() += 1;
        rsx! {
            div { "data-counting-item": "", {context.children} }
        }
    }
}

#[derive(Clone, PartialEq, Props)]
struct CountingCollectionAppProps {
    handle: Rc<RefCell<Option<FormHandle>>>,
    collection_calls: Rc<RefCell<usize>>,
    item_calls: Rc<RefCell<usize>>,
}

fn counting_collection_test_app(props: CountingCollectionAppProps) -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "tags": {
                    "type": "array",
                    "title": "Tags",
                    "maxItems": 4,
                    "items": { "type": "string", "title": "Tag", "default": "valid", "minLength": 4 }
                }
            }
        }))
        .expect("the counting-collection data schema should compile")
    });
    let form = use_form(definition, json!({ "tags": ["same", "same"] }))
        .expect("the counting-collection form should be created");
    let renderer = CountingCollection {
        collection_calls: props.collection_calls.clone(),
        item_calls: props.item_calls.clone(),
    };
    let form_to_bind = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .structure(StructureRenderers::default().with_collection(renderer))
            .build()
            .bind(&form_to_bind)
            .expect("the built-in item control should bind under the counting collection")
    });
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());

    rsx! {
        SchemaForm { form: bound, on_submit: move |_| {} }
    }
}

/// An edit inside an item re-renders the control that owns the edited node, never the item
/// host: item hosts memoize on their props, as the `CollectionRenderer` contract promises. The
/// collection itself may re-render, since the core marks the array node changed when its data
/// or findings change (`uniqueItems` depends on item values), so this test does not pin it.
#[wasm_bindgen_test]
async fn an_item_edit_does_not_re_render_the_item_hosts() {
    let root = mount_test_root();
    let handle = Rc::new(RefCell::new(None));
    let collection_calls = Rc::new(RefCell::new(0_usize));
    let item_calls = Rc::new(RefCell::new(0_usize));
    let vdom = VirtualDom::new_with_props(
        counting_collection_test_app,
        CountingCollectionAppProps {
            handle: handle.clone(),
            collection_calls: collection_calls.clone(),
            item_calls: item_calls.clone(),
        },
    );
    launch_test_vdom(&root, vdom).await;
    let form_handle = handle
        .borrow()
        .clone()
        .expect("the mounted application should expose its handle");
    let collection = poll_dom(|| {
        root.query_selector("section[data-counting-collection]")
            .expect("the collection selector should be valid")
    })
    .await;
    assert_eq!(
        collection.get_attribute("data-test-count").as_deref(),
        Some("2")
    );
    let first = input_with_binding(&root, "/tags/0");
    let element_id = collection.id();

    // Let any mount-time effects settle, then take the baseline.
    next_browser_task().await;
    let collection_baseline = *collection_calls.borrow();
    let item_baseline = *item_calls.borrow();
    assert!(collection_baseline >= 1);
    assert!(item_baseline >= 2);

    // A value that stays valid: only the control's own node changes.
    dispatch_input(&first, "samer");
    poll_dom(|| {
        (form_handle.reader().form_data().ok()? == json!({ "tags": ["samer", "same"] }))
            .then_some(())
    })
    .await;
    next_browser_task().await;
    assert_eq!(
        *item_calls.borrow(),
        item_baseline,
        "a keystroke inside an item must not re-run any item host"
    );

    // A value that fails `minLength`: the control's findings change, still inside the control.
    dispatch_input(&first, "x");
    poll_dom(|| {
        (form_handle.reader().form_data().ok()? == json!({ "tags": ["x", "same"] })).then_some(())
    })
    .await;
    poll_dom(|| (first.get_attribute("aria-invalid").as_deref() == Some("true")).then_some(()))
        .await;
    next_browser_task().await;
    assert_eq!(
        *item_calls.borrow(),
        item_baseline,
        "a validation change inside an item must not re-run any item host"
    );

    // The counters are live: a structural change re-runs the collection, and every item host
    // re-renders because its `count` prop changed.
    element_by_id(&format!("{element_id}-append")).click();
    poll_dom(|| {
        (form_handle.reader().form_data().ok()? == json!({ "tags": ["x", "same", "valid"] }))
            .then_some(())
    })
    .await;
    poll_dom(|| {
        (collection.get_attribute("data-test-count").as_deref() == Some("3")).then_some(())
    })
    .await;
    assert!(
        *collection_calls.borrow() > collection_baseline,
        "appending an item re-runs the collection renderer"
    );
    assert!(
        *item_calls.borrow() >= item_baseline + 3,
        "appending an item re-renders every item host through its count prop"
    );

    root.remove();
}

#[derive(Clone, PartialEq, Props)]
struct HookControlProps {
    context: ControlRenderContext,
}

/// A text control built on `use_text_edit` the way a UI-kit package builds one: the widget
/// carries the element id — the element the hook resynchronises — reads the hook's value, and
/// installs the hook's callbacks. Nothing of the built-in's markup is used.
#[allow(non_snake_case)]
fn HookInput(props: HookControlProps) -> Element {
    let edit = use_text_edit(&props.context);
    let presentation = props.context.presentation();
    let control = props.context.control();
    rsx! {
        label { r#for: presentation.element_id.clone(), "{presentation.label}" }
        input {
            id: presentation.element_id.clone(),
            name: control.name.clone(),
            value: "{edit.value}",
            readonly: edit.read_only,
            "aria-invalid": presentation.invalid,
            "aria-describedby": presentation.described_by(),
            "data-hook-widget": "text",
            oninput: move |event: FormEvent| edit.input.call(event.value()),
            onblur: move |_| edit.blur.call(()),
        }
        {presentation.present_help()}
        {presentation.present_findings()}
    }
}

/// A choice control built on `use_choice_edit`: a `select` carrying the element id whose option
/// values are the opaque identities, with a placeholder option selected while nothing is.
#[allow(non_snake_case)]
fn HookSelect(props: HookControlProps) -> Element {
    let edit = use_choice_edit(&props.context);
    let presentation = props.context.presentation();
    let control = props.context.control();
    let selected = edit
        .selected
        .cloned()
        .map(|identity| identity.as_str().to_owned())
        .unwrap_or_default();
    let placeholder_selected = selected.is_empty();
    let options = edit.options.clone();
    let lookup = edit.options;
    rsx! {
        label { r#for: presentation.element_id.clone(), "{presentation.label}" }
        select {
            id: presentation.element_id.clone(),
            name: control.name.clone(),
            value: selected,
            "aria-invalid": presentation.invalid,
            "data-hook-widget": "choice",
            onchange: move |event: FormEvent| {
                let identity = lookup
                    .iter()
                    .find(|option| option.identity.as_str() == event.value())
                    .map(|option| option.identity.clone());
                edit.select.call(identity);
            },
            onblur: move |_| edit.blur.call(()),
            option { value: "", disabled: true, selected: placeholder_selected, "" }
            for option in options {
                option {
                    key: "{option.identity.as_str()}",
                    value: option.identity.as_str().to_owned(),
                    disabled: option.disabled,
                    "{option.label}"
                }
            }
        }
        {presentation.present_findings()}
    }
}

/// Dispatches every control to a hook-based custom widget: choices to [`HookSelect`], the rest to
/// [`HookInput`]. The kind is definition-stable, so each node always renders the same child.
struct HookRenderer;

impl ControlRenderer for HookRenderer {
    fn render(&self, context: ControlRenderContext) -> Element {
        match context.control().kind {
            schemaform_dioxus::ControlKind::Choice => rsx! { HookSelect { context } },
            _ => rsx! { HookInput { context } },
        }
    }
}

fn hook_renderer_test_app(props: TestAppProps) -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "required": ["quantity", "secret_mode"],
            "properties": {
                "quantity": { "type": "integer", "title": "Quantity", "minimum": 0 },
                "secret_mode": {
                    "type": "string",
                    "title": "Secret mode",
                    "enum": ["private", "public"],
                    "writeOnly": true
                }
            }
        }))
        .expect("the hook-renderer data schema should compile")
    });
    let form = use_form(
        definition,
        json!({ "quantity": 1, "secret_mode": "private" }),
    )
    .expect("the hook-renderer form should be created");
    let form_to_bind = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .controls(ControlRegistry::with_builtins().matcher(
                schemaform_dioxus::BUILTIN_CONTROL_PRIORITY + 10,
                Arc::new(EveryControl),
                Arc::new(HookRenderer),
            ))
            .build()
            .bind(&form_to_bind)
            .expect("the hook renderer should bind every control")
    });
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let errors = props.errors.clone();

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot| *props.submitted.borrow_mut() = Some(snapshot),
            on_error: move |error| errors.borrow_mut().push(error),
        }
    }
}

/// The edit hooks keep their DOM contract through a custom widget, not only through the
/// built-ins: a write the core accepts reaches form data; an unparseable one stays in the widget
/// as typed behind a parse blocker; a write the core rejects is reported and the widget carrying
/// the element id is put back to the canonical text; and a write-only widget rests on its
/// placeholder after every write. A widget that put the element id on a wrapper instead of the
/// input would silently lose the last two, which is why this runs against a custom renderer.
#[wasm_bindgen_test]
async fn hook_based_custom_widgets_are_resynchronised_after_rejected_and_write_only_writes() {
    let (
        MountedTestApp {
            root, form_handle, ..
        },
        errors,
    ) = mount_test_app_with_errors(hook_renderer_test_app).await;
    let quantity = input_with_binding(&root, "/quantity");
    assert_eq!(
        quantity.get_attribute("data-hook-widget").as_deref(),
        Some("text"),
        "the custom widget renders, not the built-in"
    );
    assert_eq!(quantity.value(), "1");
    let form_data = || {
        form_handle
            .reader()
            .form_data()
            .expect("form should be readable")
    };

    // An accepted write reaches form data.
    dispatch_input(&quantity, "7");
    poll_dom(|| (form_data()["quantity"] == json!(7)).then_some(())).await;
    assert_eq!(quantity.value(), "7");

    // An unparseable write is kept as typed behind a parse blocker; the value is untouched.
    dispatch_input(&quantity, "-");
    poll_dom(|| (quantity.get_attribute("aria-invalid").as_deref() == Some("true")).then_some(()))
        .await;
    assert_eq!(quantity.value(), "-");
    assert_eq!(form_data()["quantity"], json!(7));
    assert_described_by_resolves(&quantity);
    dispatch_input(&quantity, "7");
    poll_dom(|| (quantity.get_attribute("aria-invalid").as_deref() == Some("false")).then_some(()))
        .await;

    // A write the core rejects — an edit buffer over its resource limit — is reported and the
    // widget is put back to the canonical text synchronously.
    let oversized = "9".repeat(512 * 1024 + 1);
    dispatch_input(&quantity, &oversized);
    assert_eq!(
        quantity.value(),
        "7",
        "the hook resynchronises the element carrying the element id"
    );
    assert_eq!(form_data()["quantity"], json!(7));
    assert!(
        matches!(
            errors.borrow().as_slice(),
            [HandleError::UserOperation(
                schemaform::form::UserOperationError::ResourceLimit(limit)
            )] if limit.dimension() == "edit_buffer_bytes"
        ),
        "the rejected write reaches on_error: {:?}",
        errors.borrow()
    );

    // A write-only choice rests on its placeholder after every write, accepted or not.
    let secret_mode = select_with_binding(&root, "/secret_mode");
    assert_eq!(
        secret_mode.get_attribute("data-hook-widget").as_deref(),
        Some("choice")
    );
    assert_eq!(secret_mode.value(), "");
    assert_eq!(secret_mode.selected_index(), 0);
    let public = select_options(&secret_mode)
        .into_iter()
        .find(|(_, label)| label == "public")
        .map(|(value, _)| value)
        .expect("the write-only choice offers its options");
    dispatch_select_change(&secret_mode, &public);
    assert_eq!(
        secret_mode.value(),
        "",
        "a write-only widget never shows its value"
    );
    assert_eq!(secret_mode.selected_index(), 0);
    poll_dom(|| (form_data()["secret_mode"] == json!("public")).then_some(())).await;
    assert_eq!(
        errors.borrow().len(),
        1,
        "the write-only write was accepted"
    );

    root.remove();
}

/// Returns the affordance button of `kind` that the custom renderer emitted for `input`. The
/// selector requires the adapter's id prefix, so a renderer that drops the id is caught here.
fn affordance_button(
    root: &web_sys::Element,
    input: &HtmlInputElement,
    kind: &str,
) -> web_sys::HtmlElement {
    root.query_selector(&format!(
        "button[data-affordance=\"{kind}\"][id^=\"{}-\"]",
        input.id()
    ))
    .expect("the affordance selector should be valid")
    .unwrap_or_else(|| {
        panic!(
            "{} should offer the {kind} affordance right now",
            input.id()
        )
    })
    .dyn_into::<web_sys::HtmlElement>()
    .expect("the affordance should be a button")
}

/// Lists the affordance kinds the custom renderer currently offers for `input`, in order.
fn affordance_kinds(root: &web_sys::Element, input: &HtmlInputElement) -> Vec<String> {
    let buttons = root
        .query_selector_all(&format!("button[data-affordance][id^=\"{}-\"]", input.id()))
        .expect("the affordance selector should be valid");
    (0..buttons.length())
        .filter_map(|index| buttons.item(index))
        .filter_map(|node| node.dyn_into::<web_sys::Element>().ok())
        .filter_map(|element| element.get_attribute("data-affordance"))
        .collect()
}

struct MountedTestApp {
    root: web_sys::Element,
    form_handle: FormHandle,
    submitted: Rc<RefCell<Option<SubmissionSnapshot>>>,
}

struct MountedBusinessCorpus {
    root: web_sys::Element,
    handles: Rc<RefCell<HashMap<String, FormHandle>>>,
    submitted: Rc<RefCell<HashSet<String>>>,
}

struct MountedIsolatedUpdatesTestApp {
    root: web_sys::Element,
    form_handle: FormHandle,
    lifecycle: Rc<RefCell<HashMap<InstanceIdentity, LifecycleCounts>>>,
    matcher_calls: Rc<RefCell<usize>>,
}

struct MountedProductionReactivityTestApp {
    root: web_sys::Element,
    form_handle: FormHandle,
    observations: Rc<RefCell<Vec<RenderObservation>>>,
    mounted: Signal<bool>,
}

struct MountedViewportTestApp {
    iframe: HtmlIFrameElement,
    root: web_sys::Element,
    form_handle: FormHandle,
    submitted: Rc<RefCell<Option<SubmissionSnapshot>>>,
    wide_grid_columns: (String, String),
}

async fn mount_test_app(app: fn(TestAppProps) -> Element) -> MountedTestApp {
    mount_test_app_with_errors(app).await.0
}

async fn mount_test_app_with_errors(
    app: fn(TestAppProps) -> Element,
) -> (MountedTestApp, Rc<RefCell<Vec<HandleError>>>) {
    let root = mount_test_root();
    let handle = Rc::new(RefCell::new(None));
    let submitted = Rc::new(RefCell::new(None));
    let errors = Rc::new(RefCell::new(Vec::new()));
    let vdom = VirtualDom::new_with_props(
        app,
        TestAppProps {
            handle: handle.clone(),
            submitted: submitted.clone(),
            errors: errors.clone(),
        },
    );
    launch_test_vdom(&root, vdom).await;
    let form_handle = handle
        .borrow()
        .clone()
        .expect("the mounted application should expose its handle");
    (
        MountedTestApp {
            root,
            form_handle,
            submitted,
        },
        errors,
    )
}

async fn mount_business_corpus_test_app() -> MountedBusinessCorpus {
    let root = mount_test_root();
    let handles = Rc::new(RefCell::new(HashMap::new()));
    let submitted = Rc::new(RefCell::new(HashSet::new()));
    let vdom = VirtualDom::new_with_props(
        business_corpus_test_app,
        BusinessCorpusAppProps {
            handles: handles.clone(),
            submitted: submitted.clone(),
        },
    );
    launch_test_vdom(&root, vdom).await;
    MountedBusinessCorpus {
        root,
        handles,
        submitted,
    }
}

async fn mount_test_app_in_viewport(
    app: fn(TestAppProps) -> Element,
    width: u32,
) -> MountedViewportTestApp {
    let root = mount_test_root();
    let handle = Rc::new(RefCell::new(None));
    let submitted = Rc::new(RefCell::new(None));
    let errors = Rc::new(RefCell::new(Vec::new()));
    let vdom = VirtualDom::new_with_props(
        app,
        TestAppProps {
            handle: handle.clone(),
            submitted: submitted.clone(),
            errors,
        },
    );
    launch_test_vdom(&root, vdom).await;
    let document = web_sys::window().unwrap().document().unwrap();
    let iframe: HtmlIFrameElement = document
        .create_element("iframe")
        .unwrap()
        .dyn_into()
        .unwrap();
    iframe.set_width(&width.to_string());
    iframe.set_height("600");
    document.body().unwrap().append_child(&iframe).unwrap();
    iframe
        .content_document()
        .unwrap()
        .body()
        .unwrap()
        .append_child(&root)
        .expect("the rendered form should move into the viewport document");
    let style = root
        .query_selector("style")
        .unwrap()
        .expect("the responsive form should include its grid styles");
    let frame_document = iframe.content_document().unwrap();
    let viewport_style = frame_document.create_element("style").unwrap();
    viewport_style.set_text_content(style.text_content().as_deref());
    style.remove();
    frame_document
        .query_selector("head")
        .unwrap()
        .unwrap()
        .append_child(&viewport_style)
        .expect("the grid styles should apply in the viewport document");
    poll_dom(|| {
        (iframe.content_window()?.inner_width().ok()?.as_f64() == Some(width.into())).then_some(())
    })
    .await;
    let wide_cells = root
        .query_selector_all("[data-schemaform-grid-cell]")
        .unwrap();
    let wide_first: web_sys::Element = wide_cells.get(0).unwrap().unchecked_into();
    let wide_second: web_sys::Element = wide_cells.get(1).unwrap().unchecked_into();
    let frame_window = iframe.content_window().unwrap();
    let wide_grid_columns = (
        computed_grid_column_end(&frame_window, &wide_first),
        computed_grid_column_end(&frame_window, &wide_second),
    );
    let form_handle = handle.borrow().clone().unwrap();
    MountedViewportTestApp {
        iframe,
        root,
        form_handle,
        submitted,
        wide_grid_columns,
    }
}

fn computed_grid_column_end(window: &web_sys::Window, element: &web_sys::Element) -> String {
    window
        .get_computed_style(element)
        .unwrap()
        .unwrap()
        .get_property_value("grid-column-end")
        .unwrap()
}

fn focusable_order(root: &web_sys::Element) -> Vec<String> {
    let focusable = root
        .query_selector_all("input:not([disabled]), select:not([disabled]), button:not([disabled])")
        .unwrap();
    (0..focusable.length())
        .map(|index| {
            let element: web_sys::Element = focusable.get(index).unwrap().unchecked_into();
            element
                .get_attribute("name")
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| element.text_content().unwrap_or_default())
        })
        .collect()
}

async fn mount_isolated_updates_test_app() -> MountedIsolatedUpdatesTestApp {
    let root = mount_test_root();
    let handle = Rc::new(RefCell::new(None));
    let lifecycle = Rc::new(RefCell::new(HashMap::new()));
    let matcher_calls = Rc::new(RefCell::new(0));
    let vdom = VirtualDom::new_with_props(
        isolated_updates_test_app,
        IsolatedUpdatesTestAppProps {
            handle: handle.clone(),
            lifecycle: lifecycle.clone(),
            matcher_calls: matcher_calls.clone(),
        },
    );
    launch_test_vdom(&root, vdom).await;
    let form_handle = handle
        .borrow()
        .clone()
        .expect("the mounted application should expose its handle");
    MountedIsolatedUpdatesTestApp {
        root,
        form_handle,
        lifecycle,
        matcher_calls,
    }
}

async fn mount_production_reactivity_test_app(
    scenario: browser_workload_pack::Scenario,
) -> MountedProductionReactivityTestApp {
    let root = mount_test_root();
    let handle = Rc::new(RefCell::new(None));
    let observations = Rc::new(RefCell::new(Vec::new()));
    let mounted = Rc::new(RefCell::new(None));
    let vdom = VirtualDom::new_with_props(
        production_reactivity_test_app,
        ProductionReactivityTestAppProps {
            handle: handle.clone(),
            observations: observations.clone(),
            mounted: mounted.clone(),
            scenario,
        },
    );
    launch_test_vdom(&root, vdom).await;
    let form_handle = handle
        .borrow()
        .clone()
        .expect("the mounted workload should expose its handle");
    MountedProductionReactivityTestApp {
        root,
        form_handle,
        observations,
        mounted: mounted
            .borrow()
            .to_owned()
            .expect("the mounted workload should expose its visibility signal"),
    }
}

fn mount_test_root() -> web_sys::Element {
    let document = web_sys::window()
        .expect("the browser should have a window")
        .document()
        .expect("the browser should have a document");
    let root = document
        .create_element("div")
        .expect("the test root should be created");
    document
        .body()
        .expect("the browser should have a body")
        .append_child(&root)
        .expect("the test root should mount");
    root
}

async fn launch_test_vdom(root: &web_sys::Element, vdom: VirtualDom) {
    dioxus_web::launch::launch_virtual_dom(
        vdom,
        dioxus_web::Config::new().rootelement(root.clone()),
    );
    poll_dom(|| {
        root.query_selector("form input, form select, form output, form button")
            .expect("the control selector should be valid")
    })
    .await;
}

fn assert_focused(input: &HtmlInputElement) {
    let focused = web_sys::window()
        .expect("the browser should have a window")
        .document()
        .expect("the browser should have a document")
        .active_element()
        .expect("the browser should have an active element");
    assert_eq!(focused.id(), input.id());
}

async fn wait_for_input_focus(input: &HtmlInputElement, operation: &str) {
    let expected = input.id();
    for _ in 0..100 {
        let focused = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element());
        if focused.is_some_and(|focused| focused.id() == expected) {
            return;
        }
        next_microtask().await;
    }
    let actual = web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.active_element())
        .map(|focused| focused.id())
        .unwrap_or_default();
    panic!("{operation} focused {actual}, expected {expected}");
}

async fn wait_for_summary_focus(root: &web_sys::Element) {
    let summary = root
        .query_selector("[data-finding-summary]")
        .expect("the summary selector should be valid")
        .expect("the form should expose a finding summary");
    assert_eq!(summary.get_attribute("role").as_deref(), Some("region"));
    assert_eq!(
        summary.get_attribute("aria-label").as_deref(),
        Some("Finding summary")
    );
    assert_eq!(summary.get_attribute("tabindex").as_deref(), Some("-1"));
    poll_dom(|| {
        let focused = web_sys::window()?.document()?.active_element()?;
        (focused.id() == summary.id()).then_some(())
    })
    .await;
}

fn control_with_binding(form: &FormHandle, binding: &str) -> InstanceIdentity {
    let root = form.reader().read().expect("form should be readable").root;
    let mut pending = vec![root];
    let mut observed = Vec::new();
    while let Some(identity) = pending.pop() {
        let projection = form
            .node(identity)
            .expect("the form should be readable")
            .expect("the form node should exist")
            .read()
            .expect("the form node should be readable")
            .expect("the form node should remain present");
        if projection
            .binding
            .as_ref()
            .is_some_and(|pointer| pointer.as_str() == binding)
        {
            return identity;
        }
        observed.push(
            projection
                .binding
                .as_ref()
                .map(|pointer| pointer.as_str().to_owned()),
        );
        pending.extend(projection.children.into_iter().rev());
    }
    panic!("the bound control {binding} should exist; observed {observed:?}")
}

fn browser_blocking_batch(revision: schemaform::DataRevision) -> ExternalFindingBatch {
    ExternalFindingBatch::new(
        "server",
        revision,
        [ExternalFinding::blocking(
            "server-rejected",
            JsonPointer::parse("/quantity").expect("the quantity pointer should be valid"),
            json!({}),
        )],
    )
}

fn assert_settled_lifecycle_state(form: &FormHandle, control: InstanceIdentity) {
    let form_projection = form.reader().read().expect("form should be readable");
    let control_projection = form
        .node(control)
        .expect("the form should be readable")
        .expect("the lifecycle control should remain present")
        .read()
        .expect("the lifecycle control should remain readable")
        .expect("the lifecycle control should remain present");
    assert!(!form_projection.submission_attempted);
    assert_eq!(control_projection.edit_buffer, None);
    assert_eq!(control_projection.parse_blocker, None);
    assert!(control_projection.validation_findings.is_empty());
    assert!(!control_projection.touched);
    assert!(!control_projection.dirty);
}

fn input_with_binding(root: &web_sys::Element, binding: &str) -> HtmlInputElement {
    maybe_input_with_binding(root, binding).expect("the bound input should exist")
}

/// Asserts that every id referenced by `aria-describedby` and `aria-errormessage` on `element`
/// resolves to an element in the document.
fn assert_described_by_resolves(element: &web_sys::Element) -> Vec<String> {
    let document = web_sys::window()
        .expect("the browser test should run in a window")
        .document()
        .expect("the browser test should have a document");
    let mut referenced = Vec::new();
    for attribute in ["aria-describedby", "aria-errormessage"] {
        for id in element
            .get_attribute(attribute)
            .unwrap_or_default()
            .split_whitespace()
        {
            assert!(
                document.get_element_by_id(id).is_some(),
                "{attribute} on #{} references missing element #{id}",
                element.id()
            );
            referenced.push(id.to_owned());
        }
    }
    referenced
}

fn maybe_input_with_binding(root: &web_sys::Element, binding: &str) -> Option<HtmlInputElement> {
    root.query_selector(&format!("input[name='{binding}']"))
        .ok()
        .flatten()
        .map(web_sys::Element::unchecked_into)
}

fn select_with_binding(root: &web_sys::Element, binding: &str) -> HtmlSelectElement {
    root.query_selector(&format!("select[name='{binding}']"))
        .expect("the select selector should be valid")
        .expect("the bound select should exist")
        .dyn_into()
        .expect("the choice control should be a select")
}

fn select_options(select: &HtmlSelectElement) -> Vec<(String, String)> {
    let options = select
        .query_selector_all("option")
        .expect("the choice option selector should be valid");
    (0..options.length())
        .filter_map(|index| options.get(index))
        .filter_map(|option| option.dyn_into::<web_sys::Element>().ok())
        .map(|option| {
            (
                option.get_attribute("value").unwrap_or_default(),
                option.text_content().unwrap_or_default(),
            )
        })
        .collect()
}

fn dispatch_select_change(select: &HtmlSelectElement, value: &str) {
    select.set_value(value);
    let event_init = EventInit::new();
    event_init.set_bubbles(true);
    event_init.set_cancelable(true);
    let event = Event::new_with_event_init_dict("change", &event_init)
        .expect("the select change event should be created");
    assert!(
        select
            .dispatch_event(&event)
            .expect("change should dispatch")
    );
}

fn dispatch_select_alternative(select: &HtmlSelectElement) {
    let current = select.value();
    let options = select
        .query_selector_all("option")
        .expect("the choice option selector should be valid");
    let alternative = (0..options.length())
        .filter_map(|index| options.get(index))
        .filter_map(|option| option.dyn_into::<web_sys::Element>().ok())
        .filter(|option| !option.has_attribute("disabled"))
        .filter_map(|option| option.get_attribute("value"))
        .find(|value| value != &current)
        .expect("the choice control should expose an alternative value");
    dispatch_select_change(select, &alternative);
}

fn dispatch_input(input: &HtmlInputElement, value: &str) {
    input.set_value(value);
    let event_init = InputEventInit::new();
    event_init.set_bubbles(true);
    event_init.set_cancelable(true);
    let event = InputEvent::new_with_event_init_dict("input", &event_init)
        .expect("the input event should be created");
    assert!(input.dispatch_event(&event).expect("input should dispatch"));
}

fn dispatch_paste_input(input: &HtmlInputElement, value: &str) {
    input.set_value(value);
    let event_init = InputEventInit::new();
    event_init.set_bubbles(true);
    event_init.set_cancelable(true);
    event_init.set_input_type("insertFromPaste");
    let event = InputEvent::new_with_event_init_dict("input", &event_init)
        .expect("the paste input event should be created");
    assert!(
        input
            .dispatch_event(&event)
            .expect("paste input should dispatch")
    );
}

fn dispatch_composition(input: &HtmlInputElement, event_type: &str, data: &str) {
    let event_init = CompositionEventInit::new();
    event_init.set_bubbles(true);
    event_init.set_cancelable(true);
    event_init.set_data(data);
    let event = CompositionEvent::new_with_event_init_dict(event_type, &event_init)
        .expect("the composition event should be created");
    assert!(
        input
            .dispatch_event(&event)
            .expect("composition should dispatch")
    );
}

fn dispatch_keydown(target: &web_sys::HtmlElement, key: &str) -> bool {
    let init = KeyboardEventInit::new();
    init.set_bubbles(true);
    init.set_cancelable(true);
    init.set_key(key);
    let event = KeyboardEvent::new_with_keyboard_event_init_dict("keydown", &init)
        .expect("the keyboard event should be constructed");
    target
        .dispatch_event(&event)
        .expect("the keyboard event should dispatch")
}

async fn wait_for_tab_selection(
    tab: &web_sys::HtmlElement,
    panel: &web_sys::HtmlElement,
    operation: &str,
    require_focus: bool,
) {
    let tab_id = tab.id();
    let panel_id = panel.id();
    for _ in 0..100 {
        let document = web_sys::window().unwrap().document().unwrap();
        let tab = document
            .get_element_by_id(&tab_id)
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap();
        let panel = document
            .get_element_by_id(&panel_id)
            .unwrap()
            .dyn_into::<web_sys::HtmlElement>()
            .unwrap();
        if tab.get_attribute("aria-selected").as_deref() == Some("true")
            && tab.tab_index() == 0
            && !panel.hidden()
            && (!require_focus || document.active_element().unwrap().id() == tab.id())
        {
            return;
        }
        next_microtask().await;
    }
    let document = web_sys::window().unwrap().document().unwrap();
    let tab = document.get_element_by_id(&tab_id).unwrap();
    let panel: web_sys::HtmlElement = document
        .get_element_by_id(&panel_id)
        .unwrap()
        .dyn_into()
        .unwrap();
    panic!(
        "{operation} did not select and focus tab {tab_id}: selected={:?}, tabindex={:?}, hidden={}, active={:?}",
        tab.get_attribute("aria-selected"),
        tab.get_attribute("tabindex"),
        panel.hidden(),
        document.active_element().map(|element| element.id()),
    );
}

fn dispatch_checkbox_input(input: &HtmlInputElement, checked: bool) {
    input.set_checked(checked);
    let event_init = InputEventInit::new();
    event_init.set_bubbles(true);
    event_init.set_cancelable(true);
    let event = InputEvent::new_with_event_init_dict("input", &event_init)
        .expect("the checkbox input event should be created");
    assert!(
        input
            .dispatch_event(&event)
            .expect("checkbox input should dispatch")
    );
}

fn dispatch_submit(form: &HtmlFormElement) {
    let submit_init = EventInit::new();
    submit_init.set_bubbles(true);
    submit_init.set_cancelable(true);
    let submit = Event::new_with_event_init_dict("submit", &submit_init)
        .expect("the submit event should be created");
    assert!(
        !form
            .dispatch_event(&submit)
            .expect("submit should dispatch")
    );
}

async fn accessibility_checkpoint(id: &str, trace: &str, root: &web_sys::Element) {
    use js_sys::{Function, Promise, Reflect};
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;

    let window = web_sys::window().expect("the browser should have a window");
    let hook = Reflect::get(
        window.as_ref(),
        &JsValue::from_str("__dynamicFormsAccessibilityCheckpoint"),
    )
    .expect("the accessibility hook should be readable");
    if hook.is_undefined() {
        return;
    }
    let hook: Function = hook
        .dyn_into()
        .expect("the accessibility hook should be a function");
    let promise: Promise = hook
        .call3(
            window.as_ref(),
            &JsValue::from_str(id),
            &JsValue::from_str(trace),
            root.as_ref(),
        )
        .expect("the accessibility checkpoint should start")
        .dyn_into()
        .expect("the accessibility checkpoint should return a promise");
    JsFuture::from(promise)
        .await
        .expect("the accessibility checkpoint should complete");
}

async fn next_microtask() {
    use js_sys::Promise;
    use wasm_bindgen::JsValue;
    use wasm_bindgen_futures::JsFuture;

    JsFuture::from(Promise::resolve(&JsValue::NULL))
        .await
        .expect("the browser microtask should complete");
}

async fn next_browser_task() {
    use js_sys::Promise;
    use wasm_bindgen_futures::JsFuture;

    let promise = Promise::new(&mut |resolve, _| {
        web_sys::window()
            .unwrap()
            .set_timeout_with_callback(&resolve)
            .expect("the browser task should be scheduled");
    });
    JsFuture::from(promise)
        .await
        .expect("the browser task should complete");
}

async fn poll_dom<T>(mut read: impl FnMut() -> Option<T>) -> T {
    for attempt in 0..400 {
        if let Some(value) = read() {
            return value;
        }
        if attempt % 10 == 9 {
            next_browser_task().await;
        } else {
            next_microtask().await;
        }
    }
    panic!("Dioxus did not render the expected browser state");
}
