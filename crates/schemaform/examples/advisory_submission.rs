use schemaform::{FormDefinition, form::SubmissionBlocker};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "quantity": { "type": "integer", "minimum": 1 }
        }
    }))?;
    let mut form = definition.create_form(json!({ "quantity": 0 }))?;
    let quantity = form
        .node(form.view().root())
        .and_then(|root| root.children().next())
        .expect("the generated quantity control should exist");

    // Out of range: the gated path would refuse. The advisory path hands over
    // the data as it stands, with the `minimum` finding beside it.
    let (transition, advisory) = form.prepare_advisory_submission().into_parts();
    assert!(
        !transition.is_empty(),
        "preparation marks submission attempted"
    );
    assert_eq!(advisory.form_data(), &json!({ "quantity": 0 }));
    assert_eq!(
        advisory.findings().map(describe).collect::<Vec<_>>(),
        ["minimum at /quantity"]
    );

    // Unparseable: the edit buffer never becomes form data. The member keeps
    // its prior canonical value, and a parse finding joins the validation one.
    form.user().input_text(quantity, "-")?;
    let (_, advisory) = form.prepare_advisory_submission().into_parts();
    assert_eq!(advisory.form_data(), &json!({ "quantity": 0 }));
    assert_eq!(
        advisory.findings().map(describe).collect::<Vec<_>>(),
        ["parse InvalidInteger", "minimum at /quantity"]
    );

    println!("{}", advisory.form_data());
    Ok(())
}

fn describe(blocker: &SubmissionBlocker) -> String {
    match blocker {
        SubmissionBlocker::Parse { kind, .. } => format!("parse {kind:?}"),
        SubmissionBlocker::Validation(finding) => {
            format!(
                "{} at {}",
                finding.code(),
                finding.instance_location().as_str()
            )
        }
        other => format!("{other:?}"),
    }
}
