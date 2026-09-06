//! The daisyUI control renderer: which widget each control kind and widget symbol renders, and
//! what surrounds it.

use dioxus::prelude::*;
use schemaform::{CompilationProfile, FormDefinition, JsonPointer, json::parse_ui_schema_v1};
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

use crate::support::{
    RenderedForm, TestAppProps, assert_aria_references_resolve, gallery_app, inner_html, tags,
    text_of,
};
use demo::components::schemaform_daisyui::controls;

/// The gallery form, mounted and settled.
fn mount() -> RenderedForm {
    RenderedForm::mount(gallery_app)
}

#[test]
fn string_number_and_integer_controls_render_as_daisyui_inputs() {
    let rendered = mount();

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
fn a_non_nullable_boolean_is_a_native_checkbox_with_the_daisyui_class() {
    let rendered = mount();

    let active = rendered.control("/active");
    assert_eq!(active.element, "input");
    assert_eq!(active.attribute("type"), Some("checkbox"));
    assert!(
        active.classes().contains(&"checkbox"),
        "the checkbox should carry daisyUI's class: {active:?}"
    );
    assert_eq!(active.attribute("checked"), Some("true"));
    assert_eq!(active.attribute("aria-required"), Some("true"));
    assert_eq!(active.attribute("aria-invalid"), Some("false"));
    assert_eq!(
        active.attribute("id").map(str::to_owned),
        Some(rendered.control_id("/active"))
    );
}

#[test]
fn a_nullable_boolean_is_the_registry_checkbox_showing_null_as_indeterminate() {
    let mut rendered = mount();

    let newsletter = rendered.control("/newsletter");
    assert_eq!(newsletter.element, "button");
    assert_eq!(newsletter.attribute("role"), Some("checkbox"));
    assert!(
        newsletter.classes().contains(&"checkbox"),
        "the checkbox should carry daisyUI's class: {newsletter:?}"
    );
    assert_eq!(newsletter.attribute("aria-checked"), Some("mixed"));
    assert_eq!(
        newsletter.attribute("id").map(str::to_owned),
        Some(rendered.control_id("/newsletter"))
    );

    let actions = rendered.actions_at("/newsletter");
    actions
        .set_value(json!(true))
        .expect("the boolean should be set");
    rendered.settle();

    assert_eq!(
        rendered.control("/newsletter").attribute("aria-checked"),
        Some("true")
    );
}

#[test]
fn clicking_a_null_checkbox_checks_it_and_clicking_again_unchecks_it() {
    let mut rendered = mount();
    assert_eq!(
        rendered.control("/newsletter").attribute("aria-checked"),
        Some("mixed")
    );

    // Null is the indeterminate state, and activating an indeterminate checkbox checks it, as
    // a native indeterminate checkbox and the WAI-ARIA mixed-state checkbox pattern do; the
    // first click must not land on unchecked, which daisyUI draws the same as indeterminate.
    rendered.click("/newsletter");

    assert_eq!(
        rendered.control("/newsletter").attribute("aria-checked"),
        Some("true"),
        "one click on the null checkbox should check it"
    );
    assert_eq!(
        rendered.handle.reader().form_data().unwrap()["newsletter"],
        json!(true)
    );

    rendered.click("/newsletter");

    assert_eq!(
        rendered.control("/newsletter").attribute("aria-checked"),
        Some("false")
    );
    assert_eq!(
        rendered.handle.reader().form_data().unwrap()["newsletter"],
        json!(false)
    );
}

#[test]
fn a_write_only_boolean_is_a_replacement_select_that_never_shows_its_value() {
    let rendered = mount();

    let mfa = rendered.control("/mfa");
    assert_eq!(mfa.element, "select");
    assert!(
        mfa.classes().contains(&"select"),
        "the replacement select should carry daisyUI's class: {mfa:?}"
    );
    assert_eq!(mfa.attribute("data-write-only-replacement"), Some(""));
    assert_eq!(mfa.attribute("value"), Some(""));
    let html = rendered.html();
    assert!(html.contains("Replace MFA"), "{html}");
    let options = rendered.options("/mfa");
    assert_eq!(
        options,
        vec![
            ("".to_owned(), "Choose replacement".to_owned(), true),
            ("false".to_owned(), "False".to_owned(), false),
            ("true".to_owned(), "True".to_owned(), false),
        ]
    );
}

#[test]
fn a_choice_is_a_daisyui_native_select_over_opaque_identities_with_the_null_option() {
    let mut rendered = mount();

    let plan = rendered.control("/plan");
    assert_eq!(plan.element, "select");
    assert!(
        plan.classes().contains(&"select"),
        "the select should carry daisyUI's class: {plan:?}"
    );
    assert_eq!(plan.attribute("required"), Some("true"));
    let options = rendered.options("/plan");
    let labels = options
        .iter()
        .map(|(_, label, _)| label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(labels, vec!["", "None", "starter", "team"]);
    let selected = options
        .iter()
        .filter(|(_, _, selected)| *selected)
        .map(|(_, label, _)| label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(selected, vec!["team"]);
    let values = options
        .iter()
        .skip(1)
        .map(|(value, _, _)| value.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        values.len(),
        3,
        "option values should be distinct: {options:?}"
    );
    assert!(
        !values.contains(""),
        "only the placeholder is empty: {options:?}"
    );
    assert_eq!(plan.attribute("value"), options[3].0.as_str().into());

    let actions = rendered.actions_at("/plan");
    actions.set_null().expect("the choice should accept null");
    rendered.settle();

    let selected = rendered
        .options("/plan")
        .into_iter()
        .filter(|(_, _, selected)| *selected)
        .map(|(_, label, _)| label)
        .collect::<Vec<_>>();
    assert_eq!(selected, vec!["None"]);
}

#[test]
fn a_write_only_choice_shows_the_replacement_placeholder_and_no_selection() {
    let rendered = mount();

    let recovery = rendered.control("/recovery");
    assert_eq!(recovery.element, "select");
    assert!(recovery.classes().contains(&"select"), "{recovery:?}");
    assert_eq!(recovery.attribute("data-write-only-replacement"), Some(""));
    assert_eq!(recovery.attribute("value"), Some(""));
    assert!(rendered.html().contains("Replace Recovery"));
    let options = rendered.options("/recovery");
    assert_eq!(options[0].1, "Choose replacement");
    assert!(
        options[0].2,
        "the placeholder should be selected: {options:?}"
    );
    assert!(
        options.iter().skip(1).all(|(_, _, selected)| !selected),
        "no option should be selected: {options:?}"
    );
}

#[test]
fn the_radio_widget_symbol_selects_a_daisyui_radio_group_with_one_item_per_option() {
    let mut rendered = mount();

    let group = rendered
        .find(|tag| tag.attribute("role") == Some("radiogroup"))
        .expect("a radio group should be rendered");
    let group_id = group
        .attribute("id")
        .expect("the group should carry the control's element id")
        .to_owned();
    assert_eq!(group.attribute("aria-invalid"), Some("false"));
    assert!(
        group.attribute("aria-labelledby").is_some(),
        "the group should be labelled: {group:?}"
    );

    let items = rendered.find_all(|tag| tag.attribute("role") == Some("radio"));
    assert_eq!(
        items.len(),
        3,
        "one item per option, the null option included"
    );
    for item in &items {
        assert!(item.classes().contains(&"radio"), "{item:?}");
        assert!(item.attribute("aria-labelledby").is_some(), "{item:?}");
    }
    let checked = items
        .iter()
        .filter(|item| item.attribute("aria-checked") == Some("true"))
        .count();
    assert_eq!(checked, 1);
    let checked_id = items
        .iter()
        .find(|item| item.attribute("aria-checked") == Some("true"))
        .and_then(|item| item.attribute("id"))
        .expect("the checked item should carry an id")
        .to_owned();
    assert_eq!(rendered.labelled_by_text(&checked_id), "yearly");

    // The hidden form participants carry the control's name, one per item.
    let participants = rendered.find_all(|tag| {
        tag.element == "input"
            && tag.attribute("type") == Some("radio")
            && tag.attribute("name") == Some("/billing")
    });
    assert_eq!(participants.len(), 3);

    rendered
        .actions_at("/billing")
        .set_null()
        .expect("billing should accept null");
    rendered.settle();

    let checked_id = rendered
        .find_all(|tag| tag.attribute("role") == Some("radio"))
        .iter()
        .find(|item| item.attribute("aria-checked") == Some("true"))
        .and_then(|item| item.attribute("id"))
        .expect("the null item should be checked")
        .to_owned();
    assert_eq!(rendered.labelled_by_text(&checked_id), "None");
    assert_eq!(
        rendered
            .find(|tag| tag.attribute("role") == Some("radiogroup"))
            .and_then(|tag| tag.attribute("id").map(str::to_owned)),
        Some(group_id)
    );
}

#[test]
fn a_constant_is_read_only_output_with_its_presence_affordances() {
    let rendered = mount();

    let tier = rendered.control("/tier");
    assert_eq!(tier.element, "output");
    assert_eq!(tier.attribute("data-schemaform-control"), Some("constant"));
    assert_eq!(tier.attribute("tabindex"), Some("-1"));
    assert_eq!(tier.attribute("aria-invalid"), Some("false"));
    let html = rendered.html();
    assert!(html.contains(">standard</output>"), "{html}");
    assert!(
        rendered
            .find(|tag| tag.attribute("name") == Some("/tier") && tag.element != "output")
            .is_none(),
        "a constant is never an editable widget: {html}"
    );

    let tier_id = rendered.control_id("/tier");
    let remove = rendered
        .find(|tag| tag.attribute("id") == Some(&format!("{tier_id}-remove-value")))
        .expect("an optional constant offers its remove affordance");
    assert_eq!(remove.element, "button");
    assert!(remove.classes().contains(&"btn"), "{remove:?}");
}

#[test]
fn the_select_widget_symbol_selects_the_daisyui_compound_select() {
    let mut rendered = mount();

    let trigger = rendered.control("/region");
    assert_eq!(trigger.element, "button");
    assert_eq!(trigger.attribute("aria-haspopup"), Some("listbox"));
    assert!(
        trigger.classes().contains(&"select"),
        "the trigger should carry daisyUI's class: {trigger:?}"
    );
    assert_eq!(
        trigger.attribute("id").map(str::to_owned),
        Some(rendered.control_id("/region"))
    );
    assert_eq!(trigger.attribute("aria-expanded"), Some("false"));
    assert!(
        rendered.html().contains(">eu</span></button>"),
        "the trigger should show the selected option's label"
    );

    let actions = rendered.actions_at("/region");
    actions
        .set_value(json!("us"))
        .expect("region should accept us");
    rendered.settle();
    assert!(rendered.html().contains(">us</span></button>"));

    // The null option is an option like any other: selecting null shows its label, which is the
    // adapter's localized message rather than the JSON spelling.
    actions.set_null().expect("region should accept null");
    rendered.settle();
    assert!(rendered.html().contains(">None</span></button>"));
}

#[test]
fn a_write_only_control_uses_the_password_type_and_the_replacement_label() {
    let rendered = mount();

    let secret = rendered.control("/secret");
    assert_eq!(secret.attribute("type"), Some("password"));
    assert_eq!(secret.attribute("value"), Some(""));
    assert_eq!(secret.attribute("placeholder"), Some("Choose replacement"));
    assert!(rendered.html().contains("Replace Secret"));
}

#[test]
fn a_read_only_control_renders_as_output_rather_than_an_editable_input() {
    let rendered = mount();

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
    let mut rendered = mount();
    // Surface an error so `aria-errormessage` is emitted too.
    let quantity = rendered.actions_at("/quantity");
    quantity
        .input_text("-")
        .expect("the parse blocker should be recorded");
    rendered.settle();

    let html = rendered.html();
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

    assert!(assert_aria_references_resolve(&html) > 0);
}

#[test]
fn a_null_text_control_is_an_empty_input_whose_presence_affordances_say_it_is_null() {
    let rendered = mount();

    // The input shows what the user edits, so null is nothing rather than the spelling `null`,
    // which the first keystroke would extend; the "Set Nickname" affordance beside it is how
    // the form tells null from an empty string.
    let nickname = rendered.control("/nickname");
    assert_eq!(nickname.element, "input");
    assert_eq!(nickname.attribute("value"), Some(""));
    let nickname_id = rendered.control_id("/nickname");
    assert!(
        rendered
            .by_id(&format!("{nickname_id}-set-value"))
            .is_some(),
        "a null nickname offers its set affordance"
    );
}

#[test]
fn presence_affordances_render_as_daisyui_buttons_carrying_their_ids() {
    let rendered = mount();
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

/// The findings that block submission and concern the entered value are presented in the error
/// region the control references through `aria-errormessage`, and every one of them carries the
/// adapter's stable id, so the ids the adapter hands out resolve to elements for blocking findings
/// as they do for descriptions.
#[test]
fn blocking_findings_render_in_the_error_region_with_their_stable_ids() {
    let mut rendered = mount();
    let quantity_id = rendered.control_id("/quantity");
    rendered
        .actions_at("/quantity")
        .input_text("-")
        .expect("the parse blocker should be recorded");
    rendered.settle();

    let html = rendered.html();
    let errors_id = format!("{quantity_id}-errors");
    let quantity = rendered.control("/quantity");
    assert_eq!(quantity.attribute("aria-invalid"), Some("true"));
    assert_eq!(
        quantity.attribute("aria-errormessage"),
        Some(errors_id.as_str()),
        "the control references the error region: {quantity:?}"
    );
    let region = rendered
        .by_id(&errors_id)
        .expect("the error region should be rendered");
    assert_eq!(region.attribute("aria-live"), Some("polite"));
    assert!(region.has_classes(&["text-error"]), "{region:?}");

    let findings = tags(&inner_html(&html, &errors_id))
        .into_iter()
        .filter(|tag| tag.attribute("data-finding").is_some())
        .collect::<Vec<_>>();
    assert_eq!(findings.len(), 1, "one blocking finding: {html}");
    let finding = &findings[0];
    assert_eq!(finding.attribute("data-blocking"), Some("true"));
    let finding_id = finding
        .attribute("id")
        .expect("the finding carries the adapter's stable id");
    assert!(
        finding_id.starts_with(&format!("{quantity_id}-")),
        "the stable id is derived from the control's element id: {finding_id}"
    );
    assert!(
        inner_html(&html, finding_id).contains("Enter a valid integer."),
        "{html}"
    );
    assert!(assert_aria_references_resolve(&html) > 0);

    // Once the value parses again the region is empty but still mounted, so it can announce the
    // next error.
    rendered
        .actions_at("/quantity")
        .input_text("2")
        .expect("a valid integer should apply");
    rendered.settle();
    let html = rendered.html();
    assert!(rendered.by_id(&errors_id).is_some(), "{html}");
    assert!(
        tags(&inner_html(&html, &errors_id)).is_empty(),
        "the region is empty while nothing blocks: {html}"
    );
    assert_eq!(
        rendered.control("/quantity").attribute("aria-invalid"),
        Some("false")
    );
}

/// A text control holding data it cannot edit — a number where the schema wants a string — shows
/// the data in a readout beside its replace affordance, as the built-in does, so the user sees
/// what replacement discards. The readout describes the input and carries a stable id.
#[test]
fn a_text_control_holding_incompatible_data_shows_the_readout_beside_its_replace_affordance() {
    let mut rendered = mount();
    let name_id = rendered.control_id("/name");
    let handle = rendered.handle.clone();
    rendered.drive(|| {
        handle
            .try_transact(|draft| {
                draft.set(&JsonPointer::parse("/name").expect("pointer"), json!(42));
                Ok::<_, ()>(())
            })
            .expect("the host installs incompatible data")
    });

    let html = rendered.html();
    let readout_id = format!("{name_id}-incompatible");
    let readout = rendered
        .by_id(&readout_id)
        .expect("incompatible data shows its readout");
    assert!(readout.attribute("data-incompatible-value").is_some());
    assert!(readout.has_classes(&["text-warning"]), "{readout:?}");
    assert_eq!(text_of(&html, &readout_id), "42");
    let replace = rendered
        .by_id(&format!("{name_id}-replace-value"))
        .expect("incompatible data offers replacement");
    assert_eq!(replace.element, "button");
    let name = rendered.control("/name");
    assert!(
        name.attribute("aria-describedby")
            .is_some_and(|value| value.split_whitespace().any(|id| id == readout_id)),
        "the readout describes the input: {name:?}"
    );
    assert_eq!(
        name.attribute("readonly"),
        Some("true"),
        "the core accepts no text for data it cannot edit, so the input is read-only until it is replaced: {name:?}"
    );

    let actions = rendered.actions_at("/name");
    rendered.drive(|| {
        actions
            .replace_value(json!("Ada"))
            .expect("replacement is allowed")
    });
    assert!(rendered.by_id(&readout_id).is_none());
    assert!(
        rendered
            .by_id(&format!("{name_id}-replace-value"))
            .is_none()
    );
}

/// A choice rendered as the registry's compound `Select` takes no part in native form
/// submission: the trigger carries the control binding as its `name`, and no hidden input does.
#[test]
fn the_compound_select_renders_no_hidden_form_participant() {
    let rendered = mount();

    let named = rendered.find_all(|tag| tag.attribute("name") == Some("/region"));
    assert_eq!(
        named.len(),
        1,
        "only the trigger carries the name: {named:?}"
    );
    assert_eq!(named[0].element, "button");
    assert!(
        rendered
            .find(|tag| tag.element == "input" && tag.attribute("name") == Some("/region"))
            .is_none()
    );
}

/// A form with the finding shapes the gallery does not produce: a string whose `allOf` branches
/// disagree on a title, which the compiler reports as a non-blocking capability finding on that
/// control, and a string control whose UI schema names a choice widget symbol.
fn variants_app(props: TestAppProps) -> Element {
    const UI_SCHEMA: &str = r#"{
      "version": 1,
      "root": {
        "type": "stack",
        "value": {
          "children": [
            {
              "type": "control",
              "value": { "binding": { "origin": "root", "pointer": "/note" } }
            },
            {
              "type": "control",
              "value": {
                "binding": { "origin": "root", "pointer": "/code" },
                "widget": "daisyui:radio"
              }
            }
          ]
        }
      }
    }"#;
    let definition = use_hook(|| {
        let ui_schema = parse_ui_schema_v1(UI_SCHEMA.as_bytes(), &CompilationProfile::default())
            .expect("the variants UI schema should parse");
        FormDefinition::compiler(json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "note": {
                    "allOf": [
                        { "type": "string", "title": "Text" },
                        { "type": "string", "title": "Count" }
                    ]
                },
                "code": { "type": "string", "title": "Code" }
            }
        }))
        .ui_schema(ui_schema)
        .compile()
        .expect("the variants data schema should compile")
    });
    let form = use_form(definition, json!({ "note": "hello", "code": "x1" }))
        .expect("the variants form should be created");
    props
        .handle
        .borrow_mut()
        .get_or_insert_with(|| form.clone());
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .controls(controls())
            .build()
            .bind(&form)
            .expect("the daisyUI registry should bind the variants form")
    });
    rsx! {
        SchemaForm { form: bound, on_submit: move |_| {} }
    }
}

/// A capability finding is about what the form can present, not the entered value, so it is a
/// description of the field rather than an error: it carries its stable id, describes the input,
/// and — being non-blocking — takes the warning colour and leaves the field valid. The error
/// region stays empty.
#[test]
fn an_advisory_capability_finding_is_a_warning_description_that_leaves_the_field_valid() {
    let rendered = RenderedForm::mount(variants_app);
    let html = rendered.html();

    let note = rendered.control("/note");
    let note_id = rendered.control_id("/note");
    assert_eq!(note.attribute("aria-invalid"), Some("false"), "{note:?}");
    // The built-in summary lists the same finding under its own id; the local one is the element
    // whose id the adapter derived from the control's.
    let finding = rendered
        .find(|tag| {
            tag.attribute("data-finding") == Some("annotation.conflict")
                && tag
                    .attribute("id")
                    .is_some_and(|id| id.starts_with(&format!("{note_id}-")))
        })
        .unwrap_or_else(|| panic!("the title conflict renders as a local finding: {html}"));
    assert_eq!(finding.attribute("data-blocking"), Some("false"));
    assert!(finding.has_classes(&["text-warning"]), "{finding:?}");
    let finding_id = finding.attribute("id").expect("stable id");
    assert!(
        note.attribute("aria-describedby")
            .is_some_and(|value| value.split_whitespace().any(|id| id == finding_id)),
        "the finding describes the input: {note:?}"
    );
    assert!(
        tags(&inner_html(&html, &format!("{note_id}-errors"))).is_empty(),
        "a capability finding is never a field error: {html}"
    );
    assert!(assert_aria_references_resolve(&html) > 0);
}

/// A widget symbol names how a *choice* is presented; on a control of any other kind the
/// renderer dispatches on the kind first, so the symbol changes nothing.
#[test]
fn a_choice_widget_symbol_on_a_text_control_changes_nothing() {
    let rendered = RenderedForm::mount(variants_app);

    let code = rendered.control("/code");
    assert_eq!(code.element, "input", "{code:?}");
    assert_eq!(code.attribute("type"), Some("text"));
    assert!(code.classes().contains(&"input"), "{code:?}");
    assert!(
        rendered
            .find(|tag| tag.attribute("role") == Some("radiogroup"))
            .is_none(),
        "no radio group renders for a string"
    );
}
