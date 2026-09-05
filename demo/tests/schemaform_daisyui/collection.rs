use schemaform::JsonPointer;
use serde_json::json;

use crate::support::{
    RenderedForm, Tag, arrays_app, assert_aria_references_resolve, inner_html, tags, text_of,
};

fn mount() -> RenderedForm {
    RenderedForm::mount(arrays_app)
}

/// The collection roots in document order: `tags`, then `team`.
fn collections(rendered: &RenderedForm) -> Vec<Tag> {
    rendered.find_all(|tag| tag.attribute("data-schemaform-daisyui") == Some("collection"))
}

fn id_of(tag: &Tag) -> String {
    tag.attribute("id")
        .unwrap_or_else(|| panic!("{tag:?} should carry an id"))
        .to_owned()
}

/// The collection root is a focusable, labelled fieldset carrying the adapter's element id
/// and ARIA state, with the help text described-by it, the append affordance as an outline
/// button, the container presence operations as buttons, and the live region placed inside.
/// None of the built-in's chrome or markers remain.
#[test]
fn the_collection_is_a_labelled_fieldset_with_append_presence_help_and_the_live_region() {
    let rendered = mount();
    let html = rendered.html();

    let roots = collections(&rendered);
    assert_eq!(roots.len(), 2, "{html}");
    let tags_root = &roots[0];
    let tags_id = id_of(tags_root);
    assert_eq!(tags_root.element, "fieldset");
    assert_eq!(tags_root.attribute("tabindex"), Some("-1"));
    assert_eq!(tags_root.attribute("aria-invalid"), Some("false"));
    assert!(
        tags_root.has_classes(&["fieldset", "rounded-box", "border"]),
        "{tags_root:?}"
    );
    assert_eq!(text_of(&html, &format!("{tags_id}-legend")), "Tags");
    assert_eq!(
        text_of(&html, &format!("{tags_id}-help")),
        "Keywords for the badge."
    );
    assert_eq!(
        tags_root.attribute("aria-describedby"),
        Some(&*format!("{tags_id}-help"))
    );

    let append = rendered
        .by_id(&format!("{tags_id}-append"))
        .expect("append should be offered below maxItems");
    assert_eq!(append.element, "button");
    assert_eq!(append.attribute("type"), Some("button"));
    assert!(
        append.has_classes(&["btn", "btn-sm", "btn-outline"]),
        "{append:?}"
    );
    assert!(html.contains(">Add Tags item</button>"), "{html}");

    let remove_value = rendered
        .by_id(&format!("{tags_id}-remove-value"))
        .expect("an optional present array offers remove-value");
    assert!(remove_value.has_classes(&["btn"]), "{remove_value:?}");

    let statuses = rendered.find_all(|tag| tag.attribute("data-array-status").is_some());
    assert_eq!(statuses.len(), 2, "one live region per collection: {html}");
    let root_at = html.find(&format!("id=\"{tags_id}\"")).expect("root");
    let status_at = html.find("data-array-status").expect("status");
    let team_at = html
        .find(&format!("id=\"{}\"", id_of(&roots[1])))
        .expect("team root");
    assert!(root_at < status_at && status_at < team_at, "{html}");

    for marker in [
        "class=\"schemaform-group schemaform-array\"",
        "data-schemaform-array",
        "data-append-item",
        "data-insert-item-before",
        "data-move-item-up",
        "data-move-item-down",
        "data-remove-item",
        "data-remove-value",
    ] {
        assert!(!html.contains(marker), "{marker} is the built-in's: {html}");
    }
    assert!(assert_aria_references_resolve(&html) > 0);
}

/// Each item is a card inside the adapter-owned row wrapper, named by a title that reads the
/// item noun and its position, with the item affordances as a `join` of square buttons that
/// carry their affordance ids, their positional accessible names, and their labels as titles.
/// The actions precede the item's controls; the adapter's move gating shows in which buttons
/// exist.
#[test]
fn each_item_is_a_card_with_its_affordances_as_joined_square_buttons_before_its_controls() {
    let rendered = mount();
    let html = rendered.html();

    let first = rendered.control_id("/tags/0");
    let second = rendered.control_id("/tags/1");
    for (stem, position) in [(&first, 1), (&second, 2)] {
        let row = rendered
            .by_id(&format!("{stem}-row"))
            .expect("the adapter's row wrapper should carry the row id");
        assert_eq!(row.attribute("data-array-item"), Some(""));
        let card = rendered
            .find(|tag| tag.attribute("aria-labelledby") == Some(&format!("{stem}-row-title")))
            .expect("the card should be labelled by its title");
        assert!(card.has_classes(&["card", "card-border"]), "{card:?}");
        assert_eq!(card.attribute("role"), Some("group"));
        assert_eq!(
            card.attribute("data-schemaform-daisyui"),
            Some("collection-item")
        );
        assert_eq!(
            text_of(&html, &format!("{stem}-row-title")),
            format!("Tags item {position}")
        );
        let row_at = html.find(&format!("id=\"{stem}-row\"")).expect("row");
        let card_at = html
            .find(&format!("aria-labelledby=\"{stem}-row-title\""))
            .expect("card");
        assert!(
            row_at < card_at,
            "the card is inside the row wrapper: {html}"
        );
    }

    let insert = rendered
        .by_id(&format!("{first}-insert-before"))
        .expect("insert is allowed below maxItems");
    assert_eq!(insert.element, "button");
    assert_eq!(insert.attribute("type"), Some("button"));
    assert!(
        insert.has_classes(&["btn", "btn-sm", "btn-square", "join-item"]),
        "{insert:?}"
    );
    assert_eq!(
        insert.attribute("aria-label"),
        Some("Insert Tags item before position 1")
    );
    assert_eq!(insert.attribute("title"), Some("Insert Tags item before"));
    let move_down = rendered
        .by_id(&format!("{first}-move-down"))
        .expect("the first item can move down");
    assert_eq!(
        move_down.attribute("aria-label"),
        Some("Move Tags item at position 1 down")
    );
    let remove = rendered
        .by_id(&format!("{first}-remove"))
        .expect("removal is allowed without minItems");
    assert_eq!(
        remove.attribute("aria-label"),
        Some("Remove Tags item at position 1")
    );
    assert!(
        rendered.by_id(&format!("{first}-move-up")).is_none(),
        "the first item cannot move up"
    );
    assert!(
        rendered.by_id(&format!("{second}-move-down")).is_none(),
        "the last item cannot move down"
    );
    assert_eq!(
        rendered
            .by_id(&format!("{second}-move-up"))
            .and_then(|tag| tag.attribute("aria-label").map(str::to_owned)),
        Some("Move Tags item at position 2 up".to_owned())
    );
    let joins = rendered.find_all(|tag| tag.classes().contains(&"join"));
    assert_eq!(joins.len(), 3, "one join per item, the team item included");

    // The actions come before the item's own controls, which the adapter's focus rule
    // (item root first) makes safe.
    let insert_at = html
        .find(&format!("id=\"{first}-insert-before\""))
        .expect("insert");
    let input_at = html.find("name=\"/tags/0\"").expect("the first tag input");
    assert!(insert_at < input_at, "{html}");

    // The team item is at minItems and alone: only insertion is offered, and the built-in
    // fixed object it renders sits inside the card body after the header.
    let member = rendered.control_id("/team/0/name");
    let team_item = rendered
        .find(|tag| tag.attribute("data-schemaform-fixed-object").is_some())
        .expect("the team item is a built-in fixed object");
    let team_stem = team_item
        .attribute("id")
        .expect("the item root carries the stem")
        .to_owned();
    assert_ne!(
        team_stem, member,
        "the item root and its control have distinct ids"
    );
    assert!(
        rendered
            .by_id(&format!("{team_stem}-insert-before"))
            .is_some()
    );
    for suffix in ["move-up", "move-down", "remove"] {
        assert!(
            rendered.by_id(&format!("{team_stem}-{suffix}")).is_none(),
            "{suffix} is not allowed for a sole item at minItems"
        );
    }
    let title_at = html
        .find(&format!("id=\"{team_stem}-row-title\""))
        .expect("title");
    let object_at = html
        .find(&format!("id=\"{team_stem}\""))
        .expect("object root");
    assert!(title_at < object_at, "{html}");
}

/// Appending up to `maxItems` withdraws the append and insert buttons together; removing
/// every item shows the empty state in place of the cards.
#[test]
fn append_disappears_at_max_items_and_an_emptied_collection_shows_its_empty_state() {
    let mut rendered = mount();
    let tags_id = id_of(&collections(&rendered)[0]);
    let actions = rendered.collection_actions_at("/tags");

    rendered.drive(|| actions.append().expect("a third tag fits under maxItems"));
    assert_eq!(
        rendered.control("/tags/2").attribute("value"),
        Some("fresh")
    );
    assert!(rendered.by_id(&format!("{tags_id}-append")).is_none());
    assert!(
        rendered
            .find(|tag| tag
                .attribute("id")
                .is_some_and(|id| id.ends_with("-insert-before") && id.starts_with(&tags_id)))
            .is_none(),
        "insertion is withdrawn at maxItems too"
    );
    assert!(
        rendered
            .find(|tag| tag.attribute("data-schemaform-daisyui") == Some("collection-empty"))
            .is_none()
    );

    let items = rendered
        .handle
        .node(rendered.identity_at("/tags"))
        .expect("the form should be readable")
        .expect("tags should exist")
        .read()
        .expect("the node should be readable")
        .expect("tags should be present")
        .collection_items;
    for item in items {
        rendered.drive(|| actions.remove(item.item).expect("tags has no minItems"));
    }

    let html = rendered.html();
    let empty = rendered
        .find(|tag| tag.attribute("data-schemaform-daisyui") == Some("collection-empty"))
        .expect("an emptied collection shows its empty state");
    assert!(empty.has_classes(&["border-dashed"]), "{empty:?}");
    assert!(html.contains("Nothing here yet."), "{html}");
    assert!(
        rendered
            .find(|tag| tag.attribute("data-schemaform-daisyui") == Some("collection-item"))
            .map(|card| card.attribute("aria-labelledby").map(str::to_owned))
            .is_none_or(|title| !title.is_some_and(|title| title.starts_with(&tags_id))),
        "no tag card remains: {html}"
    );
    assert!(
        rendered.by_id(&format!("{tags_id}-append")).is_some(),
        "append returns once under maxItems"
    );
}

/// The container presence operations ride on the presentation: removing the optional array
/// offers materialize with the array focused, and host-installed incompatible data shows its
/// readout beside a replace button. Array-level findings render inside the fieldset through
/// the component's presenter and describe the fieldset.
#[test]
fn presence_repair_and_local_findings_render_inside_the_fieldset() {
    let mut rendered = mount();
    let tags_id = id_of(&collections(&rendered)[0]);
    let actions = rendered.actions_at("/tags");

    rendered.drive(|| actions.remove_value().expect("tags is optional"));
    let materialize = rendered
        .by_id(&format!("{tags_id}-materialize"))
        .expect("a missing optional array offers materialize");
    assert!(materialize.has_classes(&["btn"]), "{materialize:?}");
    assert!(rendered.html().contains(">Add Tags</button>"));
    assert!(
        rendered
            .find(|tag| tag.attribute("data-schemaform-daisyui") == Some("collection-empty"))
            .is_some(),
        "a missing array has no items"
    );

    rendered.drive(|| {
        actions
            .materialize()
            .expect("the seed default restores the array")
    });
    assert_eq!(rendered.control("/tags/0").attribute("value"), Some("seed"));
    assert!(rendered.by_id(&format!("{tags_id}-materialize")).is_none());

    let handle = rendered.handle.clone();
    rendered.drive(|| {
        handle
            .try_transact(|draft| {
                draft.set(
                    &JsonPointer::parse("/tags").expect("pointer"),
                    json!("legacy"),
                );
                Ok::<_, ()>(())
            })
            .expect("the host installs incompatible data")
    });
    let html = rendered.html();
    let readout = rendered
        .find(|tag| tag.attribute("data-incompatible-value").is_some())
        .expect("incompatible data shows its readout");
    assert_eq!(readout.element, "output");
    assert!(readout.has_classes(&["text-warning"]), "{readout:?}");
    assert!(html.contains(">&#34;legacy&#34;</output>"), "{html}");
    assert!(
        rendered
            .by_id(&format!("{tags_id}-replace-value"))
            .is_some()
    );
    assert!(
        rendered
            .find(|tag| tag.attribute("data-schemaform-daisyui") == Some("collection-empty"))
            .is_none(),
        "the readout explains the absence of items; the empty state would contradict it"
    );

    // An array-level finding: the required team below minItems, made visible by a blocked
    // submission attempt, as the default finding policy shows untouched nodes' findings only
    // once submission has been attempted.
    rendered.drive(|| {
        handle
            .try_transact(|draft| {
                draft.set(&JsonPointer::parse("/team").expect("pointer"), json!([]));
                Ok::<_, ()>(())
            })
            .expect("the host empties the team");
        handle
            .prepare_submission()
            .expect("the submission attempt is prepared");
    });
    let team_root = collections(&rendered).remove(1);
    let team_id = id_of(&team_root);
    assert_eq!(team_root.attribute("aria-invalid"), Some("true"));
    let described_by = team_root
        .attribute("aria-describedby")
        .expect("the finding describes the fieldset")
        .to_owned();
    let html = rendered.html();
    let finding = tags(&inner_html(&html, &team_id))
        .into_iter()
        .find(|tag| tag.attribute("data-finding") == Some("minItems"))
        .unwrap_or_else(|| panic!("the minItems finding renders in the fieldset: {html}"));
    assert_eq!(finding.element, "p");
    assert!(finding.has_classes(&["text-error"]), "{finding:?}");
    assert_eq!(finding.attribute("data-blocking"), Some("true"));
    let finding_id = finding.attribute("id").expect("stable id");
    assert!(
        described_by.split_whitespace().any(|id| id == finding_id),
        "{described_by} should reference {finding_id}"
    );
    assert!(assert_aria_references_resolve(&html) > 0);
}
