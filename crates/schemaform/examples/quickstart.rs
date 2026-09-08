use schemaform::{FormDefinition, SubmissionOutcome, Transition};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "title": "Name", "minLength": 1 }
        }
    }))?;

    let mut form = definition.create_form(json!({ "name": "Ada" }))?;
    let name = form
        .node(form.view().root())
        .and_then(|root| root.children().next())
        .expect("the generated name control should exist");

    let transition = form.user().input_text(name, "Grace")?;
    process_transition(&transition);

    let (transition, outcome) = form.prepare_submission().into_parts();
    process_transition(&transition);
    match outcome {
        SubmissionOutcome::Ready(snapshot) => {
            assert_eq!(snapshot.form_data(), &json!({ "name": "Grace" }));
            println!("{}", snapshot.form_data());
        }
        SubmissionOutcome::Blocked(blockers) => {
            eprintln!("blocked by {} finding(s)", blockers.iter().count());
        }
    }
    Ok(())
}

fn process_transition(transition: &Transition) {
    for identity in transition.changed() {
        eprintln!("form node changed: {identity:?}");
    }
    for identity in transition.removed() {
        eprintln!("form node removed: {identity:?}");
    }
}
