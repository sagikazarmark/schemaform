use schemaform::{FormDefinition, SubmissionOutcome, Transition};
use serde_json::json;

fn main() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "title": "Name", "minLength": 1 }
        }
    }))
    .expect("the trusted data schema should compile");
    let mut form = definition
        .create_form(json!({ "name": "Ada" }))
        .expect("the form should be created");
    let name = form
        .node(form.view().root())
        .and_then(|root| root.children().next())
        .expect("the generated name control should exist");

    assert!(
        form.node(name)
            .expect("the name control should be current")
            .allowed_operations()
            .can_input_text()
    );
    let transition = form
        .user()
        .input_text(name, "Grace")
        .expect("the name should accept text input");
    process_transition(&transition);

    let (transition, outcome) = form.prepare_submission().into_parts();
    process_transition(&transition);
    match outcome {
        SubmissionOutcome::Ready(snapshot) => println!("{}", snapshot.form_data()),
        SubmissionOutcome::Blocked(blockers) => {
            eprintln!(
                "submission blocked by {} finding(s)",
                blockers.iter().count()
            )
        }
    }
}

fn process_transition(transition: &Transition) {
    for identity in transition.changed() {
        eprintln!("form node changed: {identity:?}");
    }
    for identity in transition.removed() {
        eprintln!("form node removed: {identity:?}");
    }
}
