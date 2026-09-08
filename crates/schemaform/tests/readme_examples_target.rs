//! The core README's examples are the files under `examples/`, quoted verbatim.
//! Each is compiled here as a module and run, so a README claim that stops
//! holding fails a test instead of a reader.

type ExampleResult = Result<(), Box<dyn std::error::Error>>;

const CORE_README: &str = include_str!("../README.md");

mod quickstart {
    include!("../examples/quickstart.rs");

    pub fn run() -> super::ExampleResult {
        main()
    }
}

mod advisory_submission {
    include!("../examples/advisory_submission.rs");

    pub fn run() -> super::ExampleResult {
        main()
    }
}

mod seeded_defaults {
    include!("../examples/seeded_defaults.rs");

    pub fn run() -> super::ExampleResult {
        main()
    }
}

mod finite_choices {
    include!("../examples/finite_choices.rs");

    pub fn run() -> super::ExampleResult {
        main()
    }
}

/// Every example the README quotes, by its file stem under `examples/`.
const README_EXAMPLES: [(&str, &str); 4] = [
    ("quickstart", include_str!("../examples/quickstart.rs")),
    (
        "advisory_submission",
        include_str!("../examples/advisory_submission.rs"),
    ),
    (
        "seeded_defaults",
        include_str!("../examples/seeded_defaults.rs"),
    ),
    (
        "finite_choices",
        include_str!("../examples/finite_choices.rs"),
    ),
];

#[test]
fn the_readme_quotes_each_example_file_verbatim_and_names_its_command() {
    for (name, source) in README_EXAMPLES {
        assert!(
            CORE_README.contains(&format!("```rust\n{source}```")),
            "the core README should quote examples/{name}.rs verbatim"
        );
        assert!(
            CORE_README.contains(&format!("cargo run -p schemaform --example {name}")),
            "the core README should say how to run examples/{name}.rs"
        );
    }
}

#[test]
fn the_quickstart_example_runs() {
    quickstart::run().expect("examples/quickstart.rs should run to completion");
}

#[test]
fn the_advisory_submission_example_runs() {
    advisory_submission::run().expect("examples/advisory_submission.rs should run to completion");
}

#[test]
fn the_seeded_defaults_example_runs() {
    seeded_defaults::run().expect("examples/seeded_defaults.rs should run to completion");
}

#[test]
fn the_finite_choices_example_runs() {
    finite_choices::run().expect("examples/finite_choices.rs should run to completion");
}
