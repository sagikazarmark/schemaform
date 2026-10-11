use schemaform::{AssertedFormat, FormDefinition, SubmissionOutcome, form::SubmissionBlocker};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data_schema = json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "when": { "type": "string", "format": "date-time" },
            "email": { "type": "string", "format": "email" },
            "id": { "type": "string", "format": "uuid" }
        }
    });
    // A date-time with no zone offset, as a `datetime-local` widget writes it.
    let form_data = json!({ "when": "2025-01-31T09:30", "email": "ada", "id": "x" });

    // By default `format` is an annotation: nothing here is a finding.
    let mut form = FormDefinition::compile(data_schema.clone())?.create_form(form_data.clone())?;
    let (_, outcome) = form.prepare_submission().into_parts();
    assert!(matches!(outcome, SubmissionOutcome::Ready(_)));

    // Asserting `date-time` and `email` makes both values findings; `uuid`,
    // which is not asserted, stays an annotation.
    let definition = FormDefinition::compiler(data_schema)
        .assert_formats([AssertedFormat::DateTime, AssertedFormat::Email])
        .compile()?;
    let mut form = definition.create_form(form_data)?;
    let (_, outcome) = form.prepare_submission().into_parts();
    let SubmissionOutcome::Blocked(blockers) = outcome else {
        panic!("values that do not match an asserted format block submission");
    };
    let mut failing = blockers
        .iter()
        .filter_map(|blocker| match blocker {
            SubmissionBlocker::Validation(finding) if finding.code() == "format" => {
                Some(finding.instance_location().as_str().to_owned())
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    failing.sort();
    assert_eq!(failing, ["/email", "/when"]);

    println!("{failing:?}");
    Ok(())
}
