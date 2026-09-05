//! The appearance axis: `Appearance::None` keeps daisyUI's component classes and drops every
//! layout utility this package emits, so a caller lays the form out without fighting a
//! source-order tie.

use dioxus::prelude::*;
use schemaform_dioxus::{SchemaForm, use_form};

use crate::support::{RenderedForm, Tag, TestAppProps, arrays_baseline, arrays_definition};
use demo::components::schemaform_daisyui::{Appearance, configuration_with};

/// The arrays form bound with every renderer at `Appearance::None`.
fn plain_arrays_app(props: TestAppProps) -> Element {
    let definition = use_hook(arrays_definition);
    let form = use_form(definition, arrays_baseline()).expect("the arrays form should be created");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let bound = use_hook(move || {
        configuration_with(Appearance::None)
            .bind(&form)
            .expect("the daisyUI seams should bind the arrays form")
    });
    rsx! {
        SchemaForm { form: bound, on_submit: move |_| {} }
    }
}

/// The utilities the package emits under `Appearance::Default`. None of them may appear on an
/// element the package renders under `Appearance::None`; the registry parts it composes keep
/// their own defaults, so the check is per element rather than over the whole document.
const PACKAGE_UTILITIES: &[&str] = &[
    "grid",
    "gap-1",
    "gap-2",
    "gap-3",
    "gap-4",
    "flex",
    "flex-wrap",
    "flex-col",
    "items-start",
    "items-center",
    "justify-between",
    "min-w-0",
    "w-fit",
    "p-4",
    "py-2",
    "rounded-box",
    "border",
    "border-dashed",
    "border-base-300",
    "bg-base-100",
    "bg-transparent",
    "text-xs",
    "text-sm",
    "text-start",
    "text-center",
    "text-error",
    "text-warning",
    "text-base-content/70",
    "font-medium",
    "tracking-wide",
    "uppercase",
    "size-4",
    "size-5",
    "shrink-0",
];

fn assert_no_package_utilities(tag: &Tag, what: &str) {
    let leaked = tag
        .classes()
        .into_iter()
        .filter(|class| PACKAGE_UTILITIES.contains(class))
        .collect::<Vec<_>>();
    assert!(
        leaked.is_empty(),
        "{what} still carries {leaked:?}: {tag:?}"
    );
}

#[test]
fn every_value_of_the_axis_is_listed() {
    assert_eq!(Appearance::ALL, [Appearance::Default, Appearance::None]);
    assert_eq!(Appearance::default(), Appearance::Default);
    assert_eq!(Appearance::Default.utilities("grid gap-4"), "grid gap-4");
    assert_eq!(Appearance::None.utilities("grid gap-4"), "");
}

/// Under `Appearance::None` the shell, the collection, its items and their buttons, the controls'
/// parts, and the findings keep their daisyUI component classes and semantic markers and emit no
/// layout utility. `sr-only` stays: it is what keeps a hidden label accessible, not decoration.
#[test]
fn appearance_none_keeps_component_classes_and_drops_every_layout_utility() {
    let mut rendered = RenderedForm::mount(plain_arrays_app);
    let name = rendered.actions_at("/name");
    name.input_text("A").expect("the edit should apply");
    name.blur().expect("leaving the control should apply");
    rendered.settle();

    let shell = rendered
        .find(|tag| tag.attribute("data-schemaform-daisyui") == Some("shell"))
        .expect("the shell renders");
    assert_no_package_utilities(&shell, "the shell");

    let collections =
        rendered.find_all(|tag| tag.attribute("data-schemaform-daisyui") == Some("collection"));
    assert_eq!(collections.len(), 2);
    for fieldset in &collections {
        assert!(fieldset.has_classes(&["fieldset"]), "{fieldset:?}");
        assert_no_package_utilities(fieldset, "a collection fieldset");
    }
    let tags_id = collections[0].attribute("id").expect("id").to_owned();
    let legend = rendered
        .by_id(&format!("{tags_id}-legend"))
        .expect("the legend renders");
    assert!(legend.has_classes(&["fieldset-legend"]), "{legend:?}");
    let help = rendered
        .by_id(&format!("{tags_id}-help"))
        .expect("the help renders");
    assert_no_package_utilities(&help, "the collection help");
    let append = rendered
        .by_id(&format!("{tags_id}-append"))
        .expect("append is offered");
    assert!(append.has_classes(&["btn", "btn-outline"]), "{append:?}");
    assert_no_package_utilities(&append, "the append button");

    let cards = rendered
        .find_all(|tag| tag.attribute("data-schemaform-daisyui") == Some("collection-item"));
    assert_eq!(cards.len(), 3);
    for card in &cards {
        assert!(
            card.has_classes(&["card", "card-border", "card-sm"]),
            "{card:?}"
        );
        assert_no_package_utilities(card, "an item card");
    }
    for body in rendered.find_all(|tag| tag.classes().contains(&"card-body")) {
        assert_no_package_utilities(&body, "a card body");
    }
    let first = rendered.control_id("/tags/0");
    let title = rendered
        .by_id(&format!("{first}-row-title"))
        .expect("the item title renders");
    assert_no_package_utilities(&title, "an item title");
    let insert = rendered
        .by_id(&format!("{first}-insert-before"))
        .expect("insert is offered");
    assert!(
        insert.has_classes(&["btn", "join-item", "btn-square"]),
        "{insert:?}"
    );
    for svg in rendered.find_all(|tag| tag.element == "svg") {
        assert_no_package_utilities(&svg, "an icon");
        assert_eq!(
            svg.attribute("width"),
            Some("1em"),
            "an unclassed icon keeps an intrinsic size: {svg:?}"
        );
    }
    let remove_value = rendered
        .by_id(&format!("{tags_id}-remove-value"))
        .expect("the optional array offers remove-value");
    assert!(remove_value.has_classes(&["btn"]), "{remove_value:?}");
    assert_no_package_utilities(&remove_value, "a presence button");

    // The controls' parts: the error region and its findings, the read-only output's classes.
    let name_id = rendered.control_id("/name");
    let errors = rendered
        .by_id(&format!("{name_id}-errors"))
        .expect("the error region renders");
    assert_no_package_utilities(&errors, "the error region");

    // The summary alert keeps `alert`, its tone and softness, and drops the layout utilities.
    let alert = rendered
        .find(|tag| tag.classes().contains(&"alert"))
        .expect("the summary is an alert");
    assert!(
        alert.has_classes(&["alert", "alert-soft", "alert-error"]),
        "{alert:?}"
    );
    assert_no_package_utilities(&alert, "the summary alert");
    let link = rendered
        .find(|tag| tag.element == "button" && tag.classes().contains(&"link"))
        .expect("the summary's finding is a link button");
    assert_no_package_utilities(&link, "a summary link");
    let list = rendered
        .find(|tag| tag.attribute("role") == Some("list"))
        .expect("the summary lists its findings");
    assert_no_package_utilities(&list, "the summary list");
}

/// Nothing structural changes with the axis: the same elements, ids, and markers render under
/// both values, so a caller styling `Appearance::None` targets what `Default` renders. Ids are
/// normalised for the form counter and the per-form item identities.
#[test]
fn appearance_none_renders_the_same_elements_and_ids_as_default() {
    let plain = RenderedForm::mount(plain_arrays_app);
    let styled = RenderedForm::mount(crate::support::arrays_app);
    let shape = |rendered: &RenderedForm| {
        let form_id = form_id(rendered);
        rendered
            .find_all(|_| true)
            .into_iter()
            .map(|tag| {
                (
                    tag.element.clone(),
                    tag.attribute("data-schemaform-daisyui").map(str::to_owned),
                    tag.attribute("id").map(|id| normalise_id(id, &form_id)),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&plain), shape(&styled));
}

fn form_id(rendered: &RenderedForm) -> String {
    rendered
        .find(|tag| tag.element == "form")
        .and_then(|form| form.attribute("id").map(str::to_owned))
        .expect("the form carries its id")
}

/// `id` with the form's counter replaced by `form` and every `item-<hash>` by `item`.
fn normalise_id(id: &str, form_id: &str) -> String {
    let id = id.replacen(form_id, "form", 1);
    let mut out = String::with_capacity(id.len());
    let mut rest = id.as_str();
    while let Some(at) = rest.find("item-") {
        out.push_str(&rest[..at + "item".len()]);
        let after = &rest[at + "item-".len()..];
        let hash = after
            .find(|character: char| !character.is_ascii_hexdigit())
            .unwrap_or(after.len());
        rest = &after[hash..];
    }
    out.push_str(rest);
    out
}
