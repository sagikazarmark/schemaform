use schemaform::{CompileError, Dialect, FormDefinition};
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // An elicitation schema as it can arrive on the wire: no `$schema`, and the
    // protocol defaults the root `type` to `object`, so the sender may leave it
    // out.
    let mut data_schema = json!({
        "properties": {
            "name": { "type": "string" }
        },
        "required": ["name"]
    });

    // The protocol fixes the dialect, so the host supplies it. Under plain JSON
    // Schema an untyped root still accepts any instance, and the compiler
    // refuses to guess a form for it.
    let compile = |data_schema: Value| {
        FormDefinition::compiler(data_schema)
            .default_dialect(Dialect::Draft202012)
            .compile()
    };
    let Err(CompileError::Capability(report)) = compile(data_schema.clone()) else {
        panic!("an untyped root has no single representation");
    };
    assert!(
        report
            .findings()
            .any(|finding| finding.code() == "validation.type.unconstrained")
    );

    // The host applies the protocol's default root type before compiling.
    if let Value::Object(root) = &mut data_schema {
        root.entry("type").or_insert_with(|| json!("object"));
    }
    let definition = compile(data_schema)?;
    let form = definition.create_form(json!({ "name": "Ada" }))?;

    println!("{}", form.form_data());
    Ok(())
}
