use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{
    FormHandle, RenderConfiguration, SchemaForm, use_form, use_form_with_defaults,
};
use serde_json::{Value, json};

use crate::components::{StatusLine, schemaform_daisyui};

/// Generated presentation needs only a Draft 2020-12 data schema. This one
/// exercises text, integer, boolean, choice, multiple choice, nullable,
/// constant, read-only, and write-only controls as well as validation and
/// submission. The two choices show both spellings side by side: a plain
/// `enum` whose options read as their values, and a constant choice (`oneOf`
/// of titled constants) whose options read as their titles in authored order.
/// The multiple choice is a `uniqueItems` array of titled constants: one
/// checkbox per option, members kept in option order, `minItems` left as a
/// finding rather than a disabled option. Five strings carry a `format` the
/// browser has a widget for, one per widget: `email`, `uri`, `date`,
/// `date-time`, and `time`. The controls are the built-in renderer's; only the
/// shell and the finding summary come from the demo's daisyUI component.
///
/// Four properties declare a `default` and are absent from the form data the
/// host supplies: `active`, `plan`, `priority`, and `daily_digest_at`. The
/// switch above the form picks between the library's two policies for them.
/// `use_form` never seeds defaults, so those controls start empty and the
/// required ones surface a finding on submit; `use_form_with_defaults` seeds
/// each from its `default` at creation, as the baseline, so they start filled
/// and neither touched nor dirty. Switching rebuilds the form, since the policy
/// is fixed at creation.
#[component]
pub fn GeneratedControlsExample() -> Element {
    let mut policy = use_signal(|| DefaultPolicy::LeaveAbsent);

    rsx! {
        div { class: "space-y-4",
            div {
                role: "group",
                "aria-label": "Policy for absent members with a declared default",
                class: "tabs tabs-border",
                for candidate in [DefaultPolicy::LeaveAbsent, DefaultPolicy::SeedDefaults] {
                    button {
                        r#type: "button",
                        class: if policy() == candidate { "tab tab-active" } else { "tab" },
                        "aria-pressed": policy() == candidate,
                        onclick: move |_| policy.set(candidate),
                        {candidate.label()}
                    }
                }
            }
            p { class: "text-sm text-base-content/75", {policy().description()} }
            div { key: "{policy().label()}",
                match policy() {
                    DefaultPolicy::LeaveAbsent => rsx! { UnseededGeneratedForm {} },
                    DefaultPolicy::SeedDefaults => rsx! { SeededGeneratedForm {} },
                }
            }
        }
    }
}

/// Which of the two policies the form applies to a member that is absent from
/// the host-supplied form data but whose schema declares a `default`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DefaultPolicy {
    /// The library's own position: nothing is seeded.
    LeaveAbsent,
    /// Every absent scalar is seeded from its `default` at creation.
    SeedDefaults,
}

impl DefaultPolicy {
    fn label(self) -> &'static str {
        match self {
            Self::LeaveAbsent => "Leave absent (library default)",
            Self::SeedDefaults => "Seed declared defaults",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::LeaveAbsent => {
                "use_form: the four defaulted members stay absent; submit to see the required ones reported."
            }
            Self::SeedDefaults => {
                "use_form_with_defaults: the four defaulted members are filled from their default, untouched and not dirty."
            }
        }
    }
}

/// One component per policy: the policy is fixed at creation, so the parent
/// remounts the form by key when the switch changes, and each variant calls
/// exactly one hook.
#[component]
fn UnseededGeneratedForm() -> Element {
    let definition = use_hook(definition);
    let form = use_form(definition, host_form_data())
        .expect("the generated example form should be created");
    rsx! { RenderedGeneratedForm { form } }
}

#[component]
fn SeededGeneratedForm() -> Element {
    let definition = use_hook(definition);
    let form = use_form_with_defaults(definition, host_form_data())
        .expect("the seeded generated example form should be created");
    rsx! { RenderedGeneratedForm { form } }
}

#[component]
fn RenderedGeneratedForm(form: FormHandle) -> Element {
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .structure(schemaform_daisyui::structure())
            .summary_presenter(schemaform_daisyui::findings())
            .build()
            .bind(&bound_form)
            .expect("built-in controls should bind")
    });
    let mut submitted = use_signal(String::new);
    let reset_form = form.clone();

    rsx! {
        div { class: "space-y-4",
            SchemaForm {
                form: bound,
                on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                    let mut displayed = snapshot.form_data().clone();
                    if let Some(access_token) = displayed
                        .as_object_mut()
                        .and_then(|object| object.get_mut("access_token"))
                    {
                        *access_token = Value::String("[redacted]".to_owned());
                    }
                    submitted.set(
                        serde_json::to_string_pretty(&displayed)
                            .expect("form data should serialize"),
                    );
                },
                on_error: move |error| crate::examples::report_form_error(&error),
            }
            button {
                class: "btn btn-sm btn-ghost",
                r#type: "button",
                onclick: move |_| {
                    if reset_form.reset().is_ok() {
                        submitted.set(String::new());
                    }
                },
                "Reset to baseline"
            }
            StatusLine { status: submitted.read().clone() }
        }
    }
}

/// The form data the host supplies. `active`, `plan`, `priority`, and
/// `daily_digest_at` are deliberately left out so the two policies differ
/// visibly.
fn host_form_data() -> Value {
    json!({
        "name": "Ada",
        "age": 36,
        "channels": ["email"],
        "nickname": null,
        "email": "ada@example.test",
        "homepage": "https://example.test/ada",
        "born_on": "1815-12-10",
        "last_seen": "2024-05-06T09:30",
        "account_type": "standard",
        "customer_id": "cus_1843",
        "access_token": "not-rendered"
    })
}

fn definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "age", "active", "plan", "priority", "channels", "account_type"],
        "properties": {
            "name": {
                "type": "string",
                "title": "Display name",
                "description": "At least three characters.",
                "minLength": 3
            },
            "age": {
                "type": "integer",
                "title": "Age",
                "minimum": 18
            },
            "active": {
                "type": "boolean",
                "title": "Account is active",
                "default": true
            },
            "plan": {
                "title": "Plan",
                "enum": ["starter", "team", "enterprise"],
                "default": "starter"
            },
            "priority": {
                "title": "Support priority",
                "oneOf": [
                    {
                        "const": "low",
                        "title": "Low priority",
                        "description": "Answered within the week."
                    },
                    {
                        "const": "normal",
                        "title": "Normal priority",
                        "description": "Answered within a business day."
                    },
                    {
                        "const": "high",
                        "title": "High priority",
                        "description": "Escalated within the hour."
                    }
                ],
                "default": "normal"
            },
            "channels": {
                "type": "array",
                "title": "Notification channels",
                "description": "Pick at least one.",
                "uniqueItems": true,
                "minItems": 1,
                "items": {
                    "oneOf": [
                        { "const": "email", "title": "Email" },
                        { "const": "sms", "title": "Text message" },
                        { "const": "push", "title": "Push notification" }
                    ]
                }
            },
            "nickname": {
                "type": ["string", "null"],
                "title": "Nickname"
            },
            "email": {
                "type": "string",
                "title": "Email",
                "description": "Rendered as the browser's email widget.",
                "format": "email"
            },
            "homepage": {
                "type": "string",
                "title": "Homepage",
                "description": "Rendered as the browser's URL widget.",
                "format": "uri"
            },
            "born_on": {
                "type": "string",
                "title": "Born on",
                "description": "Rendered as the browser's date picker.",
                "format": "date"
            },
            "last_seen": {
                "type": "string",
                "title": "Last seen",
                "description": "Rendered as the browser's local date-time picker; the value carries no zone offset.",
                "format": "date-time"
            },
            "daily_digest_at": {
                "type": "string",
                "title": "Daily digest at",
                "description": "Rendered as the browser's time picker.",
                "format": "time",
                "default": "08:00"
            },
            "account_type": {
                "title": "Account type",
                "const": "standard"
            },
            "customer_id": {
                "type": "string",
                "title": "Customer ID",
                "readOnly": true
            },
            "access_token": {
                "type": "string",
                "title": "Access token",
                "writeOnly": true
            }
        }
    }))
    .expect("the generated example schema should compile")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn example_schema_compiles() {
        super::definition();
    }

    #[test]
    fn the_two_policies_differ_exactly_on_the_defaulted_members() {
        let definition = super::definition();
        let plain = definition
            .create_form(super::host_form_data())
            .expect("the unseeded example form should be created");
        let seeded = definition
            .create_form_with_defaults(super::host_form_data())
            .expect("the seeded example form should be created");

        assert_eq!(plain.form_data(), &super::host_form_data());
        let mut expected = super::host_form_data();
        expected["active"] = json!(true);
        expected["plan"] = json!("starter");
        expected["priority"] = json!("normal");
        expected["daily_digest_at"] = json!("08:00");
        assert_eq!(seeded.form_data(), &expected);
    }
}
