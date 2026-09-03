//! SPIKE(#16): native exercise of the draft collection seam.
//!
//! A capturing `CollectionRenderer` records every `CollectionContext` and `CollectionItemContext`
//! it receives, then the test invokes the affordances outside the VirtualDom the way a button
//! handler would and observes the result through the form handle and `on_error`. This file is
//! throwaway with the rest of the spike branch.

#![allow(clippy::arc_with_non_send_sync)]

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use dioxus::prelude::{Element, Props, rsx, use_hook};
use dioxus_core::{NoOpMutations, ScopeId, VirtualDom, use_drop};
use schemaform::FormDefinition;
use schemaform_dioxus::{
    Affordance, AffordanceKind, CollectionContext, CollectionItemContext, CollectionRenderer,
    FormHandle, HandleError, RenderConfiguration, SchemaForm, StructureRenderers, use_form,
};
use serde_json::json;

#[derive(Clone, Default)]
struct Captured {
    collection: Option<CollectionContext>,
    /// Live item contexts keyed by row id; a child component with `use_drop` keeps this exact.
    items: HashMap<String, CollectionItemContext>,
}

type Capture = Rc<RefCell<Captured>>;

struct CapturingCollection {
    capture: Capture,
}

impl CollectionRenderer for CapturingCollection {
    fn collection(&self, context: CollectionContext) -> Element {
        self.capture.borrow_mut().collection = Some(context.clone());
        // Place items and announcement so the hosts mount; drop everything else on purpose.
        rsx! {
            div { id: context.presentation.element_id.clone(),
                {context.items}
                {context.announcement}
            }
        }
    }

    fn collection_item(&self, context: CollectionItemContext) -> Element {
        // The realistic shape: a renderer child component so hooks are available.
        let capture = self.capture.clone();
        rsx! {
            CaptureItem { context, capture }
        }
    }
}

#[derive(Clone, Props)]
struct CaptureItemProps {
    context: CollectionItemContext,
    capture: Capture,
}

impl PartialEq for CaptureItemProps {
    fn eq(&self, other: &Self) -> bool {
        self.context == other.context && Rc::ptr_eq(&self.capture, &other.capture)
    }
}

#[allow(non_snake_case)]
fn CaptureItem(props: CaptureItemProps) -> Element {
    let key = props.context.row_id.clone();
    props
        .capture
        .borrow_mut()
        .items
        .insert(key.clone(), props.context.clone());
    let capture = props.capture.clone();
    use_drop(move || {
        capture.borrow_mut().items.remove(&key);
    });
    rsx! {
        {props.context.children.clone()}
    }
}

#[derive(Clone, Props)]
struct AppProps {
    capture: Capture,
    handle: Rc<RefCell<Option<FormHandle>>>,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl PartialEq for AppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.capture, &other.capture)
            && Rc::ptr_eq(&self.handle, &other.handle)
            && Rc::ptr_eq(&self.errors, &other.errors)
    }
}

fn app(props: AppProps) -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "tags": {
                    "type": "array",
                    "title": "Tags",
                    "maxItems": 5,
                    "items": { "type": "string", "title": "Tag" }
                }
            }
        }))
        .expect("schema compiles")
    });
    let form = use_form(definition, json!({ "tags": ["a", "b"] })).expect("form created");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let capture = props.capture.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .structure(
                StructureRenderers::default().with_collection(CapturingCollection { capture }),
            )
            .build()
            .bind(&form)
            .expect("bind")
    });
    let errors = props.errors.clone();
    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |_| {},
            on_error: move |error| errors.borrow_mut().push(error),
        }
    }
}

struct Mounted {
    dom: VirtualDom,
    capture: Capture,
    handle: FormHandle,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl Mounted {
    fn mount() -> Self {
        let capture: Capture = Rc::default();
        let handle = Rc::new(RefCell::new(None));
        let errors = Rc::new(RefCell::new(Vec::new()));
        let mut dom = VirtualDom::new_with_props(
            app,
            AppProps {
                capture: capture.clone(),
                handle: handle.clone(),
                errors: errors.clone(),
            },
        );
        dom.rebuild_in_place();
        let handle = handle.borrow().clone().expect("handle exposed");
        Self {
            dom,
            capture,
            handle,
            errors,
        }
    }

    fn drive(&mut self, affordance: &Affordance) {
        let invoke = affordance.invoke;
        self.dom.in_scope(ScopeId::ROOT, move || invoke.call(()));
        for _ in 0..4 {
            self.dom.render_immediate(&mut NoOpMutations);
        }
    }

    fn tags(&self) -> Vec<String> {
        self.handle.reader().form_data().expect("readable")["tags"]
            .as_array()
            .expect("array")
            .iter()
            .map(|value| value.as_str().unwrap().to_owned())
            .collect()
    }

    fn collection(&self) -> CollectionContext {
        self.capture
            .borrow()
            .collection
            .clone()
            .expect("collection rendered")
    }

    fn items(&self) -> Vec<CollectionItemContext> {
        let mut items = self
            .capture
            .borrow()
            .items
            .values()
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by_key(|item| item.position);
        items
    }
}

fn kinds(affordances: &[Affordance]) -> Vec<AffordanceKind> {
    affordances
        .iter()
        .map(|affordance| affordance.kind)
        .collect()
}

#[test]
fn contexts_carry_positions_counts_and_pre_localized_affordances() {
    let mounted = Mounted::mount();
    let collection = mounted.collection();
    assert_eq!(collection.count, 2);
    assert_eq!(collection.item_label, "Tags item");
    assert_eq!(collection.presentation.label, "Tags");
    assert!(collection.presentation.element_id.ends_with("-array-0"));
    assert!(collection.incompatible_value.is_none());
    // `tags` is optional, so the container's remove-value presence rides on the presentation.
    assert_eq!(
        kinds(&collection.presentation.presence),
        [AffordanceKind::RemoveValue]
    );
    let remove_value = &collection.presentation.presence[0];
    assert_eq!(remove_value.label, "Remove Tags");
    assert_eq!(
        remove_value.id,
        format!("{}-remove-value", collection.presentation.element_id)
    );
    let append = collection.append.expect("append allowed below maxItems");
    assert_eq!(append.kind, AffordanceKind::Append);
    assert_eq!(append.label, "Add Tags item");
    assert_eq!(
        append.id,
        format!("{}-append", collection.presentation.element_id)
    );
    assert_eq!(append.accessible_name, None);

    let items = mounted.items();
    assert_eq!(items.len(), 2);
    let first = &items[0];
    let second = &items[1];
    assert_eq!((first.position, first.count), (1, 2));
    assert_eq!((second.position, second.count), (2, 2));
    assert_eq!(first.item_label, "Tags item");
    assert!(first.row_id.ends_with("-row"));
    assert_ne!(first.row_id, second.row_id);

    // First item: no move up; last item: no move down.
    assert!(first.move_up.is_none());
    assert!(first.move_down.is_some());
    assert!(second.move_up.is_some());
    assert!(second.move_down.is_none());

    // Ids hang off the row stem (row_id minus `-row`), matching the built-in contract.
    let stem = first.row_id.trim_end_matches("-row");
    let insert = first.insert_before.as_ref().expect("insert allowed");
    assert_eq!(insert.id, format!("{stem}-insert-before"));
    assert_eq!(insert.label, "Insert Tags item before");
    assert_eq!(
        insert.accessible_name.as_deref(),
        Some("Insert Tags item before position 1")
    );
    let remove = first.remove.as_ref().expect("remove allowed");
    assert_eq!(remove.id, format!("{stem}-remove"));
    assert_eq!(
        remove.accessible_name.as_deref(),
        Some("Remove Tags item at position 1")
    );
    let move_down = first.move_down.as_ref().unwrap();
    assert_eq!(move_down.id, format!("{stem}-move-down"));
    assert_eq!(
        move_down.accessible_name.as_deref(),
        Some("Move Tags item at position 1 down")
    );
}

#[test]
fn affordances_perform_operations_and_hosts_rerender_with_fresh_positions() {
    let mut mounted = Mounted::mount();

    let append = mounted.collection().append.unwrap();
    mounted.drive(&append);
    assert_eq!(mounted.tags(), ["a", "b", ""]);
    assert_eq!(mounted.collection().count, 3);
    let items = mounted.items();
    assert_eq!(items.len(), 3);
    assert_eq!(
        items.iter().map(|item| item.position).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert!(items.iter().all(|item| item.count == 3));
    // The former last item now has a move-down affordance.
    assert!(items[1].move_down.is_some());

    let move_down = items[0].move_down.clone().unwrap();
    mounted.drive(&move_down);
    assert_eq!(mounted.tags(), ["b", "a", ""]);

    let items = mounted.items();
    let move_up = items[1].move_up.clone().unwrap();
    mounted.drive(&move_up);
    assert_eq!(mounted.tags(), ["a", "b", ""]);

    let items = mounted.items();
    let insert = items[1].insert_before.clone().unwrap();
    mounted.drive(&insert);
    assert_eq!(mounted.tags(), ["a", "", "b", ""]);

    let items = mounted.items();
    let remove = items[3].remove.clone().unwrap();
    mounted.drive(&remove);
    assert_eq!(mounted.tags(), ["a", "", "b"]);

    // maxItems 5: two more appends then append disappears.
    let append = mounted.collection().append.unwrap();
    mounted.drive(&append);
    let append = mounted.collection().append.unwrap();
    mounted.drive(&append);
    assert_eq!(mounted.tags().len(), 5);
    assert!(mounted.collection().append.is_none());
    assert!(
        mounted
            .items()
            .iter()
            .all(|item| item.insert_before.is_none())
    );
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn affordance_failures_reach_on_error() {
    let mut mounted = Mounted::mount();
    let append = mounted.collection().append.unwrap();
    let remove = mounted.items()[0].remove.clone().unwrap();
    let handle = mounted.handle.clone();
    let append_invoke = append.invoke;
    let remove_invoke = remove.invoke;
    mounted.dom.in_scope(ScopeId::ROOT, move || {
        handle
            .try_transact(|_| {
                // The host holds the form borrow: the affordances cannot reach the core.
                append_invoke.call(());
                remove_invoke.call(());
                Ok::<_, ()>(())
            })
            .expect("the outer transaction should complete without mutation");
    });
    for _ in 0..4 {
        mounted.dom.render_immediate(&mut NoOpMutations);
    }
    assert_eq!(
        *mounted.errors.borrow(),
        vec![HandleError::BorrowConflict, HandleError::BorrowConflict]
    );
    assert_eq!(mounted.tags(), ["a", "b"]);
}
