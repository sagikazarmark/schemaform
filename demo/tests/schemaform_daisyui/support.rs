//! The native test harness the component's tests share: a form mounted in a `VirtualDom` and
//! observed through its server-rendered markup, as a browser would see it, plus a start-tag parser
//! over that markup. The harness also records which element the runtime knows each named control
//! as, so a test can dispatch a user's click to a widget's own listener rather than driving the
//! form through its handle.

use std::{
    any::Any,
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    rc::Rc,
};

use dioxus::core::{
    AttributeValue, ElementId, Event, ScopeId, Template, VirtualDom, WriteMutations,
};
use dioxus::html::{PlatformEventData, SerializedHtmlEventConverter, SerializedMouseData};
use dioxus::prelude::*;
use schemaform::{CompilationProfile, FormDefinition, InstanceIdentity, json::parse_ui_schema_v1};
use schemaform_dioxus::{
    CollectionActions, ControlActions, FormHandle, RenderConfiguration, SchemaForm, use_form,
};
use serde_json::json;

use demo::components::schemaform_daisyui::{configuration, controls};

/// A form with the two array shapes the collection renderer presents — string items and
/// fixed-object items — plus a validated string, bound through every daisyUI seam this component
/// exports (`configuration()`: the control registry, the structure bundle, and the finding
/// presenter in both slots).
///
/// `name` has a `minLength`, so leaving it too short provokes a summary finding. `tags` is
/// optional with a seed default and no `minItems`, so the container presence operations have a
/// target, it can be emptied, and `maxItems` withdraws append. `team` is required with
/// `minItems`, so its sole item cannot be removed and emptying it from the host provokes an
/// array-level finding.
pub(crate) fn arrays_app(props: TestAppProps) -> Element {
    let definition = use_hook(arrays_definition);
    let form = use_form(definition, arrays_baseline()).expect("the arrays form should be created");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let bound = use_hook(move || {
        configuration()
            .bind(&form)
            .expect("the daisyUI seams should bind the arrays form")
    });
    rsx! {
        SchemaForm { form: bound, on_submit: move |_| {} }
    }
}

/// The arrays form's data schema, compiled.
pub(crate) fn arrays_definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "team"],
        "properties": {
            "name": { "type": "string", "title": "Name", "minLength": 2 },
            "tags": {
                "type": "array",
                "title": "Tags",
                "description": "Keywords for the badge.",
                "default": ["seed"],
                "maxItems": 3,
                "items": { "type": "string", "title": "Tag", "default": "fresh" }
            },
            "team": {
                "type": "array",
                "title": "Team",
                "minItems": 1,
                "maxItems": 2,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["name"],
                    "properties": {
                        "name": { "type": "string", "title": "Member", "default": "New member" }
                    }
                }
            }
        }
    }))
    .expect("the arrays data schema should compile")
}

/// The arrays form's baseline data.
pub(crate) fn arrays_baseline() -> serde_json::Value {
    json!({
        "name": "Ada",
        "tags": ["rust", "dioxus"],
        "team": [{ "name": "Ada" }]
    })
}

/// The gallery's authored presentation: every control in data-schema order, except that the
/// billing cycle asks for the radio widget and the region for the compound select.
const GALLERY_UI_SCHEMA: &str = r#"{
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

pub(crate) fn gallery_app(props: TestAppProps) -> Element {
    let definition = use_hook(|| {
        let ui_schema =
            parse_ui_schema_v1(GALLERY_UI_SCHEMA.as_bytes(), &CompilationProfile::default())
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

/// Props of a test application: a slot the application fills with its form handle on first
/// render, so a test can drive the form the way a host would.
#[derive(Clone, Props)]
pub(crate) struct TestAppProps {
    pub(crate) handle: Rc<RefCell<Option<FormHandle>>>,
}

impl PartialEq for TestAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.handle, &other.handle)
    }
}

/// The rendered form: the form handle plus the markup, observed as a browser would see it.
pub(crate) struct RenderedForm {
    dom: VirtualDom,
    pub(crate) handle: FormHandle,
    /// The dynamic text attributes of every element the DOM currently holds, by the element id
    /// the runtime dispatches events to.
    elements: Elements,
}

/// A mutation writer that keeps only what a test needs to target an element for an event: the
/// dynamic text attributes the DOM set on it and the events it listens for, until the DOM
/// removes it. Static template attributes never reach a writer, so an element is found by an
/// attribute the renderer computed, such as `name`.
#[derive(Default)]
struct Elements {
    attributes: HashMap<ElementId, BTreeMap<&'static str, String>>,
    listeners: HashMap<ElementId, Vec<&'static str>>,
}

impl Elements {
    /// The element carrying `name="{name}"` that listens for `event`.
    fn named_listening(&self, name: &str, event: &str) -> Option<ElementId> {
        self.attributes
            .iter()
            .filter(|(_, attributes)| attributes.get("name").is_some_and(|value| value == name))
            .map(|(id, _)| *id)
            .find(|id| {
                self.listeners
                    .get(id)
                    .is_some_and(|events| events.contains(&event))
            })
    }

    fn forget(&mut self, id: ElementId) {
        self.attributes.remove(&id);
        self.listeners.remove(&id);
    }
}

impl WriteMutations for Elements {
    fn append_children(&mut self, _id: ElementId, _m: usize) {}
    fn assign_node_id(&mut self, _path: &'static [u8], _id: ElementId) {}
    fn create_placeholder(&mut self, _id: ElementId) {}
    fn create_text_node(&mut self, _value: &str, _id: ElementId) {}
    fn load_template(&mut self, _template: Template, _index: usize, _id: ElementId) {}
    fn replace_node_with(&mut self, id: ElementId, _m: usize) {
        self.forget(id);
    }
    fn replace_placeholder_with_nodes(&mut self, _path: &'static [u8], _m: usize) {}
    fn insert_nodes_after(&mut self, _id: ElementId, _m: usize) {}
    fn insert_nodes_before(&mut self, _id: ElementId, _m: usize) {}
    fn set_attribute(
        &mut self,
        name: &'static str,
        _ns: Option<&'static str>,
        value: &AttributeValue,
        id: ElementId,
    ) {
        let attributes = self.attributes.entry(id).or_default();
        match value {
            AttributeValue::Text(value) => {
                attributes.insert(name, value.clone());
            }
            AttributeValue::None => {
                attributes.remove(name);
            }
            _ => {}
        }
    }
    fn set_node_text(&mut self, _value: &str, _id: ElementId) {}
    fn create_event_listener(&mut self, name: &'static str, id: ElementId) {
        self.listeners.entry(id).or_default().push(name);
    }
    fn remove_event_listener(&mut self, name: &'static str, id: ElementId) {
        if let Some(events) = self.listeners.get_mut(&id) {
            events.retain(|event| *event != name);
        }
    }
    fn remove_node(&mut self, id: ElementId) {
        self.forget(id);
    }
    fn push_root(&mut self, _id: ElementId) {}
}

impl RenderedForm {
    /// Mounts `app` and settles it.
    pub(crate) fn mount(app: fn(TestAppProps) -> Element) -> Self {
        let handle = Rc::new(RefCell::new(None));
        let mut dom = VirtualDom::new_with_props(
            app,
            TestAppProps {
                handle: handle.clone(),
            },
        );
        let mut elements = Elements::default();
        dom.rebuild(&mut elements);
        let handle = handle
            .borrow()
            .clone()
            .expect("the test app should expose its form handle");
        let mut rendered = Self {
            dom,
            handle,
            elements,
        };
        rendered.settle();
        rendered
    }

    /// Field parts register their ids while they render and `Field` syncs metadata in an
    /// effect, so the control's ARIA references land on the renders that follow.
    pub(crate) fn settle(&mut self) {
        for _ in 0..4 {
            self.dom.render_immediate(&mut self.elements);
        }
    }

    /// Clicks the element carrying `name="{name}"` that listens for clicks, the way a browser
    /// dispatches a user's click to its listener, then settles the DOM.
    pub(crate) fn click(&mut self, name: &str) {
        let id = self
            .elements
            .named_listening(name, "click")
            .unwrap_or_else(|| panic!("an element named {name} should listen for clicks"));
        dioxus::html::set_event_converter(Box::new(SerializedHtmlEventConverter));
        let data: Rc<dyn Any> = Rc::new(PlatformEventData::new(Box::new(
            SerializedMouseData::default(),
        )));
        self.dom
            .runtime()
            .handle_event("click", Event::new(data, true), id);
        self.settle();
    }

    pub(crate) fn html(&self) -> String {
        dioxus_ssr::render(&self.dom)
    }

    /// Runs `operation` inside the Dioxus runtime, the way an event handler would, then settles
    /// the DOM. Collection operations announce through adapter-owned signals and so need the
    /// runtime; scalar control actions do not.
    pub(crate) fn drive<R>(&mut self, operation: impl FnOnce() -> R) -> R {
        let result = self.dom.in_scope(ScopeId::ROOT, operation);
        self.settle();
        result
    }

    /// The first tag `accept` accepts, in document order.
    pub(crate) fn find(&self, accept: impl Fn(&Tag) -> bool) -> Option<Tag> {
        tags(&self.html()).into_iter().find(accept)
    }

    /// Every tag `accept` accepts, in document order.
    pub(crate) fn find_all(&self, accept: impl Fn(&Tag) -> bool) -> Vec<Tag> {
        tags(&self.html()).into_iter().filter(accept).collect()
    }

    /// The tag carrying `id`.
    pub(crate) fn by_id(&self, id: &str) -> Option<Tag> {
        self.find(|tag| tag.attribute("id") == Some(id))
    }

    /// The text of the element that `aria-labelledby` of the element with `id` references.
    pub(crate) fn labelled_by_text(&self, id: &str) -> String {
        let label_id = self
            .by_id(id)
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
    pub(crate) fn options(&self, name: &str) -> Vec<(String, String, bool)> {
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
    pub(crate) fn control(&self, name: &str) -> Tag {
        let html = self.html();
        tags(&html)
            .into_iter()
            .find(|tag| tag.attribute("name") == Some(name))
            .unwrap_or_else(|| panic!("a control named {name} should be rendered:\n{html}"))
    }

    /// The DOM id the adapter assigned to the control bound at `name`.
    pub(crate) fn control_id(&self, name: &str) -> String {
        self.control(name)
            .attribute("id")
            .unwrap_or_else(|| panic!("the control named {name} should carry an id"))
            .to_owned()
    }

    /// The scalar-control actions of the node bound at `pointer`, for driving it through the
    /// form handle the way a host would.
    pub(crate) fn actions_at(&self, pointer: &str) -> ControlActions {
        self.handle
            .node(self.identity_at(pointer))
            .expect("the form should be readable")
            .expect("the node should exist")
            .actions()
    }

    /// The collection actions of the array bound at `pointer`.
    pub(crate) fn collection_actions_at(&self, pointer: &str) -> CollectionActions {
        self.handle
            .node(self.identity_at(pointer))
            .expect("the form should be readable")
            .expect("the node should exist")
            .collection_actions()
    }

    /// The instance identity of the node bound at `pointer`.
    pub(crate) fn identity_at(&self, pointer: &str) -> InstanceIdentity {
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
                .is_some_and(|binding| binding.as_str() == pointer)
            {
                return identity;
            }
            pending.extend(projection.children);
        }
        panic!("a node bound at {pointer} should exist");
    }
}

/// One start tag from the rendered markup.
#[derive(Debug)]
pub(crate) struct Tag {
    pub(crate) element: String,
    attributes: Vec<(String, String)>,
}

impl Tag {
    pub(crate) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub(crate) fn classes(&self) -> Vec<&str> {
        self.attribute("class")
            .map(|class| class.split_whitespace().collect())
            .unwrap_or_default()
    }

    /// Whether the tag carries every class in `expected`.
    pub(crate) fn has_classes(&self, expected: &[&str]) -> bool {
        let classes = self.classes();
        expected.iter().all(|class| classes.contains(class))
    }
}

/// Every start tag in `html`, with its attributes. Dioxus SSR writes text values in double
/// quotes with the quote character escaped, so a quote always ends such a value, and writes
/// boolean values bare (`required=true`).
pub(crate) fn tags(html: &str) -> Vec<Tag> {
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

/// The markup inside the element carrying `id`: from the end of its start tag to its matching
/// end tag, found by counting the element's own nested start and end tags.
pub(crate) fn inner_html(html: &str, id: &str) -> String {
    let start = html
        .find(&format!("id=\"{id}\""))
        .unwrap_or_else(|| panic!("an element with id {id} should exist:\n{html}"));
    let tag_start = html[..start]
        .rfind('<')
        .expect("the id sits in a start tag");
    let element = html[tag_start + 1..]
        .split(|character: char| character.is_whitespace() || character == '>')
        .next()
        .expect("a start tag names its element")
        .to_owned();
    let open = html[start..].find('>').expect("the start tag should close") + start + 1;
    let start_tag = format!("<{element}");
    let end_tag = format!("</{element}>");
    let mut depth = 1;
    let mut cursor = open;
    while depth > 0 {
        let next_open = html[cursor..].find(&start_tag).map(|at| at + cursor);
        let next_close = html[cursor..]
            .find(&end_tag)
            .map(|at| at + cursor)
            .unwrap_or_else(|| panic!("the element {id} should close:\n{html}"));
        match next_open {
            Some(at) if at < next_close => {
                depth += 1;
                cursor = at + start_tag.len();
            }
            _ => {
                depth -= 1;
                if depth == 0 {
                    return html[open..next_close].to_owned();
                }
                cursor = next_close + end_tag.len();
            }
        }
    }
    unreachable!("the loop returns when the element closes")
}

/// The text content directly inside the first element carrying `id`, up to its first child tag.
pub(crate) fn text_of(html: &str, id: &str) -> String {
    let start = html
        .find(&format!("id=\"{id}\""))
        .unwrap_or_else(|| panic!("an element with id {id} should exist:\n{html}"));
    let rest = &html[start..];
    let text_start = rest.find('>').expect("the tag should close") + 1;
    let text_end = rest[text_start..]
        .find('<')
        .expect("the element should close");
    rest[text_start..text_start + text_end].to_owned()
}

/// Asserts that every id referenced by an ARIA relationship attribute or a `for` in `html`
/// resolves to an element, and returns how many references were checked.
pub(crate) fn assert_aria_references_resolve(html: &str) -> usize {
    let ids = ids(html);
    let mut references = 0;
    for tag in tags(html) {
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
    references
}
