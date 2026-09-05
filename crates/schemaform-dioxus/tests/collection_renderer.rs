//! Native contract tests for the homogeneous-array collection seam.
//!
//! Each test configures a capturing `CollectionRenderer` through `RenderConfigurationBuilder`,
//! mounts `SchemaForm` in a native `VirtualDom`, and drives the affordances the renderer received
//! outside the VirtualDom the way a button handler would. Observations go through the form handle,
//! the captured contexts, and the host's `on_error` callback only.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use dioxus::prelude::{Element, Props, rsx, use_drop, use_hook};
use dioxus_core::{NoOpMutations, ScopeId, VirtualDom};
use schemaform::{FormDefinition, JsonPointer};
use schemaform_dioxus::{
    Affordance, AffordanceKind, CollectionContext, CollectionItemContext, CollectionRenderer,
    FormHandle, HandleError, RenderConfiguration, SchemaForm, StructureRenderers, use_form,
};
use serde_json::json;

#[derive(Clone, Default)]
struct Captured {
    collection: Option<CollectionContext>,
    /// Live item contexts keyed by row id. Each item is rendered through a child component whose
    /// `use_drop` removes its entry, so the map mirrors the mounted item hosts exactly.
    items: HashMap<String, CollectionItemContext>,
}

type Capture = Rc<RefCell<Captured>>;

struct CapturingCollection {
    capture: Capture,
}

impl CollectionRenderer for CapturingCollection {
    fn collection(&self, context: CollectionContext) -> Element {
        self.capture.borrow_mut().collection = Some(context.clone());
        rsx! {
            div { id: context.presentation.element_id.clone(),
                {context.items}
                {context.announcement}
            }
        }
    }

    fn collection_item(&self, context: CollectionItemContext) -> Element {
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
struct CollectionAppProps {
    capture: Capture,
    handle: Rc<RefCell<Option<FormHandle>>>,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl PartialEq for CollectionAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.capture, &other.capture)
            && Rc::ptr_eq(&self.handle, &other.handle)
            && Rc::ptr_eq(&self.errors, &other.errors)
    }
}

fn collection_app(props: CollectionAppProps) -> Element {
    let definition = use_hook(|| {
        FormDefinition::compile(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "tags": {
                    "type": "array",
                    "title": "Tags",
                    "default": ["seed"],
                    "maxItems": 5,
                    "items": { "type": "string", "title": "Tag" }
                }
            }
        }))
        .expect("the collection data schema should compile")
    });
    let form = use_form(definition, json!({ "tags": ["a", "b"] }))
        .expect("the collection form should be created");
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
            .expect("the built-in item control should bind under a custom collection")
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

struct MountedCollection {
    dom: VirtualDom,
    capture: Capture,
    handle: FormHandle,
    errors: Rc<RefCell<Vec<HandleError>>>,
}

impl MountedCollection {
    fn mount() -> Self {
        let capture: Capture = Rc::default();
        let handle = Rc::new(RefCell::new(None));
        let errors = Rc::new(RefCell::new(Vec::new()));
        let mut dom = VirtualDom::new_with_props(
            collection_app,
            CollectionAppProps {
                capture: capture.clone(),
                handle: handle.clone(),
                errors: errors.clone(),
            },
        );
        dom.rebuild_in_place();
        let handle = handle
            .borrow()
            .clone()
            .expect("the collection app should expose its form handle");
        Self {
            dom,
            capture,
            handle,
            errors,
        }
    }

    /// Invokes `affordance` the way an event handler would, then settles the DOM.
    fn drive(&mut self, affordance: &Affordance) {
        let affordance = affordance.clone();
        self.dom
            .in_scope(ScopeId::ROOT, move || affordance.invoke());
        self.settle();
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.dom.render_immediate(&mut NoOpMutations);
        }
    }

    fn tags(&self) -> Vec<String> {
        self.handle
            .reader()
            .form_data()
            .expect("the form should be readable")["tags"]
            .as_array()
            .expect("tags should be an array")
            .iter()
            .map(|value| value.as_str().expect("tags are strings").to_owned())
            .collect()
    }

    fn collection(&self) -> CollectionContext {
        self.capture
            .borrow()
            .collection
            .clone()
            .expect("the collection renderer should have been called")
    }

    /// The live item contexts in position order.
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
fn the_contexts_carry_positions_counts_and_pre_localized_affordances() {
    let mounted = MountedCollection::mount();

    let collection = mounted.collection();
    assert_eq!(collection.count, 2);
    assert_eq!(collection.item_label, "Tags item");
    assert_eq!(collection.presentation.label, "Tags");
    assert!(collection.presentation.element_id.ends_with("-array-0"));
    assert_eq!(collection.presentation.incompatible_value, None);
    // `tags` is optional and present, so the container offers remove-value on its presentation.
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
    assert_eq!(remove_value.accessible_name, None);
    let append = collection
        .append
        .expect("append should be allowed below maxItems");
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

    // The adapter gates moves by position: the first item cannot move up, the last cannot move
    // down.
    assert!(first.move_up.is_none());
    assert!(first.move_down.is_some());
    assert!(second.move_up.is_some());
    assert!(second.move_down.is_none());

    // Item affordance ids hang off the row stem (`row_id` minus `-row`) and carry positional
    // accessible names.
    let stem = first.row_id.trim_end_matches("-row");
    let insert = first
        .insert_before
        .as_ref()
        .expect("insert should be allowed below maxItems");
    assert_eq!(insert.kind, AffordanceKind::InsertBefore);
    assert_eq!(insert.id, format!("{stem}-insert-before"));
    assert_eq!(insert.label, "Insert Tags item before");
    assert_eq!(
        insert.accessible_name.as_deref(),
        Some("Insert Tags item before position 1")
    );
    let remove = first.remove.as_ref().expect("remove should be allowed");
    assert_eq!(remove.kind, AffordanceKind::RemoveItem);
    assert_eq!(remove.id, format!("{stem}-remove"));
    assert_eq!(remove.label, "Remove Tags item");
    assert_eq!(
        remove.accessible_name.as_deref(),
        Some("Remove Tags item at position 1")
    );
    let move_down = first.move_down.as_ref().expect("checked above");
    assert_eq!(move_down.kind, AffordanceKind::MoveDown);
    assert_eq!(move_down.id, format!("{stem}-move-down"));
    assert_eq!(move_down.label, "Move Tags item down");
    assert_eq!(
        move_down.accessible_name.as_deref(),
        Some("Move Tags item at position 1 down")
    );
    let move_up = second.move_up.as_ref().expect("checked above");
    assert_eq!(move_up.kind, AffordanceKind::MoveUp);
    assert_eq!(
        move_up.id,
        format!("{}-move-up", second.row_id.trim_end_matches("-row"))
    );
    assert_eq!(move_up.label, "Move Tags item up");
    assert_eq!(
        move_up.accessible_name.as_deref(),
        Some("Move Tags item at position 2 up")
    );
    assert!(mounted.errors.borrow().is_empty());
}

#[test]
fn item_and_append_affordances_perform_their_operations_and_hosts_re_render_with_fresh_positions() {
    let mut mounted = MountedCollection::mount();

    let append = mounted.collection().append.expect("append allowed");
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
    // The former last item gained a move-down affordance.
    assert!(items[1].move_down.is_some());

    let move_down = items[0]
        .move_down
        .clone()
        .expect("first item can move down");
    mounted.drive(&move_down);
    assert_eq!(mounted.tags(), ["b", "a", ""]);

    let items = mounted.items();
    let move_up = items[1].move_up.clone().expect("second item can move up");
    mounted.drive(&move_up);
    assert_eq!(mounted.tags(), ["a", "b", ""]);

    let items = mounted.items();
    let insert = items[1]
        .insert_before
        .clone()
        .expect("insert allowed below maxItems");
    mounted.drive(&insert);
    assert_eq!(mounted.tags(), ["a", "", "b", ""]);

    let items = mounted.items();
    let remove = items[3].remove.clone().expect("remove allowed");
    mounted.drive(&remove);
    assert_eq!(mounted.tags(), ["a", "", "b"]);

    // maxItems 5: two more appends, then append and insert-before disappear together.
    let append = mounted.collection().append.expect("append allowed");
    mounted.drive(&append);
    let append = mounted.collection().append.expect("append allowed");
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
fn a_held_form_borrow_during_an_affordance_surfaces_borrow_conflict_through_on_error() {
    let mut mounted = MountedCollection::mount();
    let append = mounted.collection().append.expect("append allowed");
    let remove = mounted.items()[0].remove.clone().expect("remove allowed");
    let handle = mounted.handle.clone();

    mounted.dom.in_scope(ScopeId::ROOT, move || {
        handle
            .try_transact(|_| {
                // The host holds the form borrow: the affordances cannot reach the core.
                append.invoke();
                remove.invoke();
                Ok::<_, ()>(())
            })
            .expect("the outer transaction should complete without mutation");
    });
    mounted.settle();

    assert_eq!(
        *mounted.errors.borrow(),
        vec![HandleError::BorrowConflict, HandleError::BorrowConflict]
    );
    assert_eq!(mounted.tags(), ["a", "b"]);
}

/// An affordance belongs to the scope that computed it. Once that scope is gone — here, the item
/// host of a removed item — invoking a retained affordance performs nothing and reports
/// `StaleAffordance` through `on_error`, rather than reaching a dropped callback. A moved item
/// keeps its host, so its retained affordances stay live and act on the same item.
#[test]
fn a_retained_affordance_of_a_removed_item_reports_stale_instead_of_acting() {
    let mut mounted = MountedCollection::mount();
    let items = mounted.items();
    let first_remove = items[0].remove.clone().expect("remove allowed");
    let second_move_up = items[1]
        .move_up
        .clone()
        .expect("the second item can move up");

    // Moving keeps the host: the affordance retained before the move still acts on its item.
    mounted.drive(&second_move_up);
    assert_eq!(mounted.tags(), ["b", "a"]);
    assert!(mounted.errors.borrow().is_empty());

    // Removing the item drops its host, and with it the scope its affordances came from. The
    // retained remove still targets item `a`, now at position 2.
    mounted.drive(&first_remove);
    assert_eq!(mounted.tags(), ["b"]);
    assert_eq!(mounted.items().len(), 1);

    mounted.drive(&first_remove);
    assert_eq!(*mounted.errors.borrow(), vec![HandleError::StaleAffordance]);
    assert_eq!(mounted.tags(), ["b"], "a stale affordance performs nothing");
    assert_eq!(mounted.items().len(), 1);
}

#[test]
fn container_presence_rides_on_the_presentation_and_repairs_missing_and_incompatible_data() {
    let mut mounted = MountedCollection::mount();

    // Remove the optional array through its presence affordance; materialize is then offered.
    let remove_value = mounted.collection().presentation.presence[0].clone();
    assert_eq!(remove_value.kind, AffordanceKind::RemoveValue);
    mounted.drive(&remove_value);
    assert_eq!(
        mounted.handle.reader().form_data().expect("readable"),
        json!({})
    );
    let collection = mounted.collection();
    assert_eq!(collection.count, 0);
    assert!(mounted.items().is_empty());
    assert_eq!(
        kinds(&collection.presentation.presence),
        [AffordanceKind::Materialize]
    );
    let materialize = collection.presentation.presence[0].clone();
    assert_eq!(materialize.label, "Add Tags");
    assert_eq!(
        materialize.id,
        format!("{}-materialize", collection.presentation.element_id)
    );
    assert_eq!(collection.presentation.incompatible_value, None);

    mounted.drive(&materialize);
    assert_eq!(mounted.tags(), ["seed"]);
    assert_eq!(mounted.collection().count, 1);

    // Host-installed incompatible data surfaces on the presentation with a replace affordance.
    let handle = mounted.handle.clone();
    mounted.dom.in_scope(ScopeId::ROOT, move || {
        handle
            .try_transact(|draft| {
                draft.set(&JsonPointer::parse("/tags").unwrap(), json!("legacy"));
                Ok::<_, ()>(())
            })
            .expect("the host should install incompatible array data");
    });
    mounted.settle();
    let collection = mounted.collection();
    assert_eq!(collection.count, 0);
    assert_eq!(
        collection.presentation.incompatible_value.as_deref(),
        Some("\"legacy\"")
    );
    let replace = collection
        .presentation
        .presence
        .iter()
        .find(|affordance| affordance.kind == AffordanceKind::Replace)
        .cloned()
        .expect("an incompatible array should offer replacement");
    assert_eq!(replace.label, "Replace Tags");
    assert_eq!(
        replace.id,
        format!("{}-replace-value", collection.presentation.element_id)
    );

    mounted.drive(&replace);
    assert_eq!(mounted.tags(), ["seed"]);
    assert_eq!(mounted.collection().presentation.incompatible_value, None);
    assert!(mounted.errors.borrow().is_empty());
}
