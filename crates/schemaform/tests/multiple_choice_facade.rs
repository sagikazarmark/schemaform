//! Multiple choice: a `uniqueItems` array whose items are a finite choice keeps
//! compiling to a homogeneous array, and additionally reports the multiple-choice
//! facet so a renderer can present one checkbox per option and toggle members by
//! value.

use schemaform::{
    CompileError, FormDefinition, InstanceIdentity, JsonPointer, SubmissionOutcome, WidgetSymbol,
    definition::{DefinitionNodeKind, SemanticKind},
    form::{UserOperationError, ValidationOutcomeView},
    ui::v1::{Binding, Control, Element as UiElement, Stack, UiSchema},
};
use serde_json::{Value, json};

/// A closed root object with one `tags` array of `enum` items and the given extra
/// array keywords.
fn tags_schema(array_keywords: Value) -> Value {
    let mut tags = json!({
        "title": "Tags",
        "description": "Pick any that apply.",
        "type": "array",
        "items": { "enum": ["alpha", "beta", "gamma"] }
    });
    for (key, value) in array_keywords
        .as_object()
        .expect("array keywords are an object")
    {
        tags[key] = value.clone();
    }
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": { "tags": tags }
    })
}

/// Every option of a node as `(value, label)`.
fn options(form: &schemaform::Form, identity: InstanceIdentity) -> Vec<(Value, String)> {
    form.node(identity)
        .expect("the node should exist")
        .definition()
        .choice_options()
        .map(|option| (option.value().clone(), option.label().to_owned()))
        .collect()
}

fn control_with_binding(form: &schemaform::Form, binding: &str) -> InstanceIdentity {
    let mut pending = vec![form.view().root()];
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).expect("the form node should exist");
        if node
            .binding()
            .is_some_and(|current| current.pointer().as_str() == binding)
        {
            return identity;
        }
        pending.extend(node.children());
    }
    panic!("the bound control should exist")
}

#[test]
fn a_unique_items_array_of_enum_items_is_a_multiple_choice_and_stays_an_array() {
    let definition = FormDefinition::compile(tags_schema(json!({ "uniqueItems": true })))
        .expect("the multiple choice should compile");
    assert_eq!(
        definition.capability_findings().count(),
        0,
        "recognition adds no capability finding"
    );
    let form = definition
        .create_form(json!({ "tags": ["beta"] }))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    let node = form.node(tags).expect("the array should exist");
    let array = node.definition();

    assert_eq!(array.kind(), DefinitionNodeKind::Control);
    assert_eq!(
        array.semantic_kind(),
        Some(SemanticKind::HomogeneousArray),
        "the node remains an array node"
    );
    assert!(array.is_multiple_choice());
    assert!(
        !array.is_choice_selectable(),
        "a multiple choice is not a scalar selection; renderers dispatch on the array facet"
    );
    assert_eq!(
        options(&form, tags),
        [
            (json!("alpha"), "alpha".to_owned()),
            (json!("beta"), "beta".to_owned()),
            (json!("gamma"), "gamma".to_owned()),
        ],
        "the array node carries the item options"
    );
    assert_eq!(array.label(), "Tags");
    assert_eq!(array.help(), Some("Pick any that apply."));

    let items = node.children().collect::<Vec<_>>();
    assert_eq!(items.len(), 1, "identity per current item keeps working");
    let item = form.node(items[0]).expect("the item should exist");
    assert_eq!(
        item.definition().semantic_kind(),
        Some(SemanticKind::Choice)
    );
    assert!(
        !item.definition().is_multiple_choice(),
        "the item template is a scalar choice, not a multiple choice"
    );
}

#[test]
fn without_unique_items_the_same_items_stay_a_plain_homogeneous_array() {
    let definition =
        FormDefinition::compile(tags_schema(json!({}))).expect("the array should compile");
    let form = definition
        .create_form(json!({ "tags": ["beta"] }))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    let array = form
        .node(tags)
        .expect("the array should exist")
        .definition();

    assert_eq!(array.semantic_kind(), Some(SemanticKind::HomogeneousArray));
    assert!(!array.is_multiple_choice());
    assert_eq!(options(&form, tags), [], "a plain array carries no options");
}

#[test]
fn unique_items_without_a_finite_item_choice_is_not_a_multiple_choice() {
    for items in [
        json!({ "type": "string" }),
        json!({ "type": "integer" }),
        json!({ "const": "only" }),
        json!({ "type": "null" }),
    ] {
        let mut schema = tags_schema(json!({ "uniqueItems": true }));
        schema["properties"]["tags"]["items"] = items.clone();
        let definition = FormDefinition::compile(schema).expect("the array should compile");
        let form = definition
            .create_form(json!({}))
            .expect("the form should be created");
        let tags = control_with_binding(&form, "/tags");
        let array = form
            .node(tags)
            .expect("the array should exist")
            .definition();
        assert_eq!(array.semantic_kind(), Some(SemanticKind::HomogeneousArray));
        assert!(
            !array.is_multiple_choice(),
            "items {items} enumerate nothing to choose from"
        );
    }
}

#[test]
fn unique_items_with_a_constant_choice_is_a_multiple_choice_in_authored_order() {
    let mut schema = tags_schema(json!({ "uniqueItems": true }));
    schema["properties"]["tags"]["items"] = json!({
        "oneOf": [
            { "const": "push", "title": "Push notification" },
            { "const": "email", "title": "Email", "description": "Daily digest." }
        ]
    });
    let definition = FormDefinition::compile(schema).expect("the array should compile");
    let form = definition
        .create_form(json!({}))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    let array = form
        .node(tags)
        .expect("the array should exist")
        .definition();

    assert!(array.is_multiple_choice());
    assert_eq!(
        options(&form, tags),
        [
            (json!("push"), "Push notification".to_owned()),
            (json!("email"), "Email".to_owned()),
        ]
    );
    assert_eq!(
        array
            .choice_options()
            .map(|option| option.description().map(str::to_owned))
            .collect::<Vec<_>>(),
        [None, Some("Daily digest.".to_owned())]
    );
}

#[test]
fn checking_options_appends_by_value_in_option_order_not_check_order() {
    let definition = FormDefinition::compile(tags_schema(json!({ "uniqueItems": true })))
        .expect("the multiple choice should compile");
    let mut form = definition
        .create_form(json!({ "tags": [] }))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    let node = form.node(tags).expect("the array should exist");
    assert!(node.allowed_operations().can_toggle_choice());
    assert!(
        node.allowed_operations().can_append_item(),
        "the collection actions stay intact"
    );

    let transition = form
        .user()
        .toggle_choice(tags, json!("gamma"))
        .expect("checking gamma should append it");
    assert_eq!(form.form_data(), &json!({ "tags": ["gamma"] }));
    assert!(transition.changed().any(|identity| identity == tags));
    let gamma = form
        .node(tags)
        .expect("the array should remain")
        .children()
        .next()
        .expect("the appended item exists");
    assert!(
        transition.changed().any(|identity| identity == gamma),
        "the new item's identity is announced as changed"
    );

    form.user()
        .toggle_choice(tags, json!("alpha"))
        .expect("checking alpha should insert it");
    assert_eq!(
        form.form_data(),
        &json!({ "tags": ["alpha", "gamma"] }),
        "checking gamma then alpha yields option order, not check order"
    );
    let node = form.node(tags).expect("the array should remain");
    assert_eq!(
        node.children()
            .map(|item| form.node(item).unwrap().current_data().cloned())
            .collect::<Vec<_>>(),
        [Some(json!("alpha")), Some(json!("gamma"))]
    );
    assert!(
        node.children().nth(1) == Some(gamma),
        "the gamma item keeps its identity while its index shifts"
    );

    form.user()
        .toggle_choice(tags, json!("beta"))
        .expect("checking beta should insert it between");
    assert_eq!(
        form.form_data(),
        &json!({ "tags": ["alpha", "beta", "gamma"] })
    );
    assert!(form.node(tags).unwrap().is_dirty());

    assert_eq!(
        form.user()
            .toggle_choice(tags, json!("delta"))
            .expect_err("a value that is no option cannot be toggled"),
        UserOperationError::OperationNotAllowed
    );
    assert_eq!(
        form.form_data(),
        &json!({ "tags": ["alpha", "beta", "gamma"] })
    );

    form.user()
        .append_item(tags)
        .expect("the collection actions stay available on the array node");
    assert_eq!(
        form.form_data(),
        &json!({ "tags": ["alpha", "beta", "gamma", "alpha"] }),
        "append seeds the first option like any scalar array"
    );
}

#[test]
fn toggling_is_not_offered_on_a_plain_array() {
    let plain = FormDefinition::compile(tags_schema(json!({}))).expect("the array should compile");
    let mut form = plain
        .create_form(json!({ "tags": [] }))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    assert!(
        !form
            .node(tags)
            .unwrap()
            .allowed_operations()
            .can_toggle_choice()
    );
    assert_eq!(
        form.user()
            .toggle_choice(tags, json!("alpha"))
            .expect_err("a plain array has no toggle"),
        UserOperationError::OperationNotAllowed
    );
}

#[test]
fn checking_an_option_on_an_absent_array_creates_it_as_typing_into_an_absent_scalar_does() {
    let definition = FormDefinition::compile(tags_schema(json!({ "uniqueItems": true })))
        .expect("the multiple choice should compile");
    let mut form = definition
        .create_form(json!({}))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    let node = form.node(tags).unwrap();
    let operations = node.allowed_operations();
    assert!(
        operations.can_toggle_choice(),
        "an absent multiple choice can be toggled, as an absent scalar can be typed into"
    );
    assert!(
        operations.can_materialize(),
        "the container's explicit presence operation stays offered while absent"
    );
    assert!(
        !operations.can_remove_value(),
        "there is nothing to remove while the array is absent"
    );
    assert_eq!(node.selected_choices().count(), 0);
    assert_eq!(node.children().count(), 0);
    assert!(!node.is_dirty());

    let transition = form
        .user()
        .toggle_choice(tags, json!("beta"))
        .expect("checking beta should create the array with that one member");
    assert_eq!(form.form_data(), &json!({ "tags": ["beta"] }));
    assert_ne!(
        transition.before_data_revision(),
        transition.after_data_revision(),
        "creation is a data transition"
    );
    assert!(transition.changed().any(|identity| identity == tags));
    let node = form.node(tags).unwrap();
    let beta = node
        .children()
        .next()
        .expect("the created array holds one item");
    assert!(
        transition.changed().any(|identity| identity == beta),
        "the new item's identity is announced as changed, as when a present array grows"
    );
    assert_eq!(transition.removed().count(), 0);
    assert!(node.is_dirty());
    assert!(
        !node.is_touched(),
        "creation by toggle is an edit; only blur marks the node touched"
    );
    assert_eq!(
        node.selected_choices()
            .map(|option| option.value().clone())
            .collect::<Vec<_>>(),
        [json!("beta")]
    );
    let operations = node.allowed_operations();
    assert!(operations.can_toggle_choice());
    assert!(!operations.can_materialize());
    assert!(operations.can_remove_value());

    form.user()
        .toggle_choice(tags, json!("beta"))
        .expect("unchecking the last member should be accepted");
    assert_eq!(
        form.form_data(),
        &json!({ "tags": [] }),
        "unchecking the last option leaves the empty array, which is distinct from absent"
    );
    let node = form.node(tags).unwrap();
    assert!(node.is_dirty());
    assert!(node.allowed_operations().can_toggle_choice());
    assert!(
        node.allowed_operations().can_remove_value(),
        "the array's remove-value presence operation is how absence is reached"
    );

    form.user()
        .remove_value(tags)
        .expect("the optional array can be removed like any optional leaf");
    assert_eq!(form.form_data(), &json!({}));
    let node = form.node(tags).unwrap();
    assert!(!node.is_dirty(), "back at the baseline");
    assert!(
        node.allowed_operations().can_toggle_choice(),
        "and the next check creates it again"
    );
}

#[test]
fn a_read_only_absent_multiple_choice_refuses_the_toggle() {
    let definition = FormDefinition::compile(tags_schema(json!({
        "uniqueItems": true,
        "readOnly": true
    })))
    .expect("the multiple choice should compile");
    let mut form = definition
        .create_form(json!({}))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    let node = form.node(tags).unwrap();
    assert!(node.is_read_only());
    assert!(!node.allowed_operations().can_toggle_choice());
    assert!(!node.allowed_operations().can_materialize());
    assert_eq!(
        form.user()
            .toggle_choice(tags, json!("alpha"))
            .expect_err("read-only refuses the toggle, absent or not"),
        UserOperationError::OperationNotAllowed
    );
    assert_eq!(form.form_data(), &json!({}), "nothing was created");
}

/// The codes of every validation finding located at `binding`, whatever the
/// visibility policy says about presenting them right now.
fn finding_codes(form: &schemaform::Form, binding: &str) -> Vec<String> {
    match form.view().validation_outcome() {
        ValidationOutcomeView::Valid => Vec::new(),
        ValidationOutcomeView::Invalid { findings, .. } => findings
            .iter()
            .filter(|finding| finding.instance_location().as_str() == binding)
            .map(|finding| finding.code().to_owned())
            .collect(),
        ValidationOutcomeView::Indeterminate(reason) => panic!("indeterminate: {reason:?}"),
    }
}

#[test]
fn unchecking_a_duplicated_member_removes_every_item_carrying_it() {
    let definition = FormDefinition::compile(tags_schema(json!({ "uniqueItems": true })))
        .expect("the multiple choice should compile");
    let mut form = definition
        .create_form(json!({ "tags": ["beta", "alpha", "beta"] }))
        .expect("data that violates uniqueItems is still editable");
    let tags = control_with_binding(&form, "/tags");
    assert_eq!(
        finding_codes(&form, "/tags"),
        ["uniqueItems"],
        "the core's duplicate finding is kept, not silently repaired"
    );
    let node = form.node(tags).expect("the array should exist");
    assert_eq!(
        node.selected_choices()
            .map(|option| option.value().clone())
            .collect::<Vec<_>>(),
        [json!("alpha"), json!("beta")],
        "the duplicated option is selected once, in option order"
    );
    let items = node.children().collect::<Vec<_>>();
    let alpha = items[1];

    let transition = form
        .user()
        .toggle_choice(tags, json!("beta"))
        .expect("unchecking beta should remove both items");
    assert_eq!(form.form_data(), &json!({ "tags": ["alpha"] }));
    let removed = transition.removed().collect::<Vec<_>>();
    assert_eq!(removed, [items[0], items[2]]);
    assert!(
        transition.changed().any(|identity| identity == alpha),
        "the surviving item shifted and is announced as changed"
    );
    assert_eq!(
        form.node(tags).unwrap().children().collect::<Vec<_>>(),
        [alpha],
        "the surviving item keeps its identity"
    );
    assert_eq!(finding_codes(&form, "/tags"), Vec::<String>::new());
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
}

#[test]
fn length_bounds_stay_findings_and_never_gate_toggling() {
    let definition = FormDefinition::compile(tags_schema(json!({
        "uniqueItems": true,
        "minItems": 1,
        "maxItems": 2
    })))
    .expect("the multiple choice should compile");
    let mut form = definition
        .create_form(json!({ "tags": ["alpha", "beta"] }))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    let operations = form.node(tags).unwrap().allowed_operations();
    assert!(
        !operations.can_append_item(),
        "maxItems still gates the collection's append"
    );
    assert!(operations.can_toggle_choice(), "but not toggling");

    form.user()
        .toggle_choice(tags, json!("gamma"))
        .expect("a third option can be checked past maxItems");
    assert_eq!(
        form.form_data(),
        &json!({ "tags": ["alpha", "beta", "gamma"] })
    );
    assert_eq!(finding_codes(&form, "/tags"), ["maxItems"]);
    assert!(matches!(
        form.prepare_submission().outcome(),
        SubmissionOutcome::Blocked(_)
    ));

    for value in ["alpha", "beta", "gamma"] {
        form.user()
            .toggle_choice(tags, json!(value))
            .expect("every option can be unchecked, past minItems too");
    }
    assert_eq!(form.form_data(), &json!({ "tags": [] }));
    assert_eq!(finding_codes(&form, "/tags"), ["minItems"]);
    let operations = form.node(tags).unwrap().allowed_operations();
    assert!(
        !operations.can_remove_item(),
        "minItems still gates the collection's remove"
    );
    assert!(operations.can_toggle_choice());
}

#[test]
fn a_member_that_is_no_option_is_incompatible_data_offered_for_replacement() {
    let definition = FormDefinition::compile(tags_schema(json!({ "uniqueItems": true })))
        .expect("the multiple choice should compile");
    let mut form = definition
        .create_form(json!({ "tags": ["gamma", "zeta", "alpha"] }))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    let node = form.node(tags).expect("the array should exist");
    let operations = node.allowed_operations();
    assert!(
        operations.can_replace_value(),
        "a stray member is offered for replacement like incompatible scalar data"
    );
    assert!(
        operations.can_toggle_choice(),
        "the recognised members remain toggleable meanwhile"
    );
    assert_eq!(
        node.selected_choices()
            .map(|option| option.value().clone())
            .collect::<Vec<_>>(),
        [json!("alpha"), json!("gamma")],
        "the stray value is neither dropped nor invented as an option"
    );
    assert_eq!(
        finding_codes(&form, "/tags/1"),
        ["enum"],
        "the stray member keeps its own finding, located at the item"
    );
    form.user()
        .blur(tags)
        .expect("the array node can be blurred");
    let node = form.node(tags).expect("the array should exist");
    assert_eq!(
        node.validation_findings()
            .map(|finding| (
                finding.code().to_owned(),
                finding.instance_location().as_str().to_owned(),
            ))
            .collect::<Vec<_>>(),
        [("enum".to_owned(), "/tags/1".to_owned())],
        "a multiple choice presents its items' findings itself, since it presents no item"
    );
    let plain_items = form
        .node(tags)
        .unwrap()
        .children()
        .map(|item| {
            form.node(item)
                .unwrap()
                .validation_findings()
                .map(|finding| finding.code().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        plain_items,
        [vec![], vec!["enum".to_owned()], vec![]],
        "the item node keeps the finding too, for a presentation that instantiates items"
    );

    let compatible = node
        .selected_choices()
        .map(|option| option.value().clone())
        .collect::<Vec<_>>();
    form.user()
        .replace_value(tags, Value::Array(compatible))
        .expect("replacing with the compatible members repairs the array");
    assert_eq!(form.form_data(), &json!({ "tags": ["alpha", "gamma"] }));
    let operations = form.node(tags).unwrap().allowed_operations();
    assert!(!operations.can_replace_value());
    assert!(operations.can_toggle_choice());

    let plain = FormDefinition::compile(tags_schema(json!({}))).expect("the array should compile");
    let form = plain
        .create_form(json!({ "tags": ["gamma", "zeta"] }))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    assert!(
        !form
            .node(tags)
            .unwrap()
            .allowed_operations()
            .can_replace_value(),
        "a plain array of rows keeps editing the stray item in its own row"
    );
}

#[test]
fn blurring_the_array_marks_it_touched_and_reveals_its_findings() {
    let definition = FormDefinition::compile(tags_schema(json!({
        "uniqueItems": true,
        "minItems": 1
    })))
    .expect("the multiple choice should compile");
    let mut form = definition
        .create_form(json!({ "tags": [] }))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    let node = form.node(tags).unwrap();
    assert!(!node.is_touched());
    assert_eq!(
        node.validation_findings().count(),
        0,
        "under the default policy an untouched node presents nothing yet"
    );

    let transition = form
        .user()
        .blur(tags)
        .expect("an array node can be blurred");
    assert!(transition.changed().any(|identity| identity == tags));
    let node = form.node(tags).unwrap();
    assert!(node.is_touched());
    assert_eq!(
        node.validation_findings()
            .map(|finding| finding.code().to_owned())
            .collect::<Vec<_>>(),
        ["minItems"]
    );

    form.reset();
    assert!(
        !form.node(tags).unwrap().is_touched(),
        "reset forgets that the array was blurred"
    );
}

#[test]
fn an_authored_control_binds_a_multiple_choice_without_an_item_template() {
    let chips = WidgetSymbol::parse("company:chips").expect("a valid widget symbol");
    let tags_control = || {
        Control::new(Binding::root(
            JsonPointer::parse("/tags").expect("a valid pointer"),
        ))
    };
    let definition = FormDefinition::compiler(tags_schema(json!({ "uniqueItems": true })))
        .ui_schema(UiSchema::new(UiElement::Stack(Stack::new([
            UiElement::Control(tags_control().widget(chips.clone())),
        ]))))
        .compile()
        .expect("a multiple choice is authored like any other control");
    let mut form = definition
        .create_form(json!({ "tags": ["beta"] }))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");
    let node = form.node(tags).expect("the authored node should exist");
    assert_eq!(node.definition().widget(), Some(&chips));
    assert!(node.definition().is_multiple_choice());
    assert_eq!(
        node.children().count(),
        1,
        "the compiled item template is instantiated per member behind the authored control"
    );

    form.user()
        .toggle_choice(tags, json!("alpha"))
        .expect("toggling works through the authored node");
    assert_eq!(form.form_data(), &json!({ "tags": ["alpha", "beta"] }));

    // A plain array still needs its item template authored.
    let plain = FormDefinition::compiler(tags_schema(json!({})))
        .ui_schema(UiSchema::new(UiElement::Stack(Stack::new([
            UiElement::Control(tags_control()),
        ]))))
        .compile();
    match plain {
        Err(CompileError::Input(_)) => {}
        Err(error) => panic!("unexpected error {error}"),
        Ok(_) => panic!("a plain array control without an item template is rejected"),
    }
}

#[test]
fn a_null_option_toggles_like_any_other_and_insertion_leaves_existing_items_in_place() {
    let mut schema = tags_schema(json!({ "uniqueItems": true }));
    schema["properties"]["tags"]["items"] = json!({
        "anyOf": [
            { "const": null, "title": "Anywhere" },
            { "const": "eu", "title": "Europe" },
            { "const": "us", "title": "United States" }
        ]
    });
    let definition = FormDefinition::compile(schema).expect("the array should compile");
    let mut form = definition
        .create_form(json!({ "tags": ["us", "mars", "eu"] }))
        .expect("the form should be created");
    let tags = control_with_binding(&form, "/tags");

    form.user()
        .toggle_choice(tags, Value::Null)
        .expect("the null option is a member like any other");
    assert_eq!(
        form.form_data(),
        &json!({ "tags": [null, "us", "mars", "eu"] }),
        "the new member goes before the first later option; out-of-order items and the \\
         stray keep their places"
    );
    let node = form.node(tags).unwrap();
    assert_eq!(
        node.selected_choices()
            .map(|option| option.label().to_owned())
            .collect::<Vec<_>>(),
        ["Anywhere", "Europe", "United States"]
    );

    form.user()
        .toggle_choice(tags, Value::Null)
        .expect("the null member can be removed again");
    assert_eq!(form.form_data(), &json!({ "tags": ["us", "mars", "eu"] }));
}
