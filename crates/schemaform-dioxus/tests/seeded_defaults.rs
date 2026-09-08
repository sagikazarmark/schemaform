//! Native contract test for creating a form handle with seeded defaults.
//!
//! The hook is mounted in a native `VirtualDom`; observations go through the returned handle.

use std::{cell::RefCell, rc::Rc};

use dioxus::prelude::{Element, Props, rsx, use_hook};
use dioxus_core::VirtualDom;
use schemaform::FormDefinition;
use schemaform_dioxus::{FormHandle, NodeProjection, use_form, use_form_with_defaults};
use serde_json::json;

type CapturedHandles = Rc<RefCell<Vec<FormHandle>>>;

#[derive(Clone, Props)]
struct SeedingAppProps {
    handles: CapturedHandles,
}

impl PartialEq for SeedingAppProps {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.handles, &other.handles)
    }
}

fn definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string", "default": "Ada" },
            "quantity": { "type": "integer", "default": 3 }
        }
    }))
    .expect("the seeding data schema should compile")
}

fn seeding_app(props: SeedingAppProps) -> Element {
    let definition = use_hook(definition);
    let plain = use_form(definition.clone(), json!({ "name": "Lin" }))
        .expect("the plain form should be created");
    let seeded = use_form_with_defaults(definition, json!({ "name": "Lin" }))
        .expect("the seeded form should be created");
    let mut handles = props.handles.borrow_mut();
    if handles.is_empty() {
        handles.push(plain);
        handles.push(seeded);
    }
    rsx! {}
}

#[test]
fn use_form_with_defaults_seeds_absent_scalars_where_use_form_does_not() {
    let handles: CapturedHandles = Rc::default();
    let mut dom = VirtualDom::new_with_props(
        seeding_app,
        SeedingAppProps {
            handles: handles.clone(),
        },
    );
    dom.rebuild_in_place();

    let handles = handles.borrow();
    let [plain, seeded] = handles.as_slice() else {
        panic!("both hooks should have produced a handle");
    };
    assert_eq!(
        plain
            .reader()
            .form_data()
            .expect("the plain form data should be readable"),
        json!({ "name": "Lin" })
    );
    assert_eq!(
        seeded
            .reader()
            .form_data()
            .expect("the seeded form data should be readable"),
        json!({ "name": "Lin", "quantity": 3 })
    );
    let quantity = control_with_binding(seeded, "/quantity");
    assert!(!quantity.touched, "seeding should not touch the control");
    assert!(!quantity.dirty, "seeded data should be the baseline");
}

fn control_with_binding(handle: &FormHandle, binding: &str) -> NodeProjection {
    let root = handle
        .reader()
        .read()
        .expect("the form should project")
        .root;
    let mut pending = vec![root];
    while let Some(identity) = pending.pop() {
        let projection = handle
            .node(identity)
            .expect("the node should be readable")
            .expect("the node should be in the form tree")
            .read()
            .expect("the node should project")
            .expect("the node should still be in the form tree");
        if projection
            .binding
            .as_ref()
            .is_some_and(|current| current.as_str() == binding)
        {
            return projection;
        }
        pending.extend(projection.children);
    }
    panic!("the bound control {binding} should exist")
}
