//! Constant choices: a `oneOf` or `anyOf` whose every branch is one annotated
//! scalar constant compiles to the labeled choice control `enum` produces,
//! with per-option titles, descriptions, and authored order.

use schemaform::{
    CompileError, FormDefinition, InstanceIdentity, JsonPointer, SubmissionOutcome,
    definition::{DefinitionNodeKind, SemanticKind},
    form::{SubmissionBlocker, UserOperationError, ValidationOutcomeView},
};
use serde_json::{Value, json};

fn object_schema(property: Value) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["priority"],
        "properties": { "priority": property }
    })
}

/// Every option of a choice control as `(value, label)`.
fn options(form: &schemaform::Form, identity: InstanceIdentity) -> Vec<(Value, String)> {
    annotated_options(form, identity)
        .into_iter()
        .map(|(value, label, _)| (value, label))
        .collect()
}

/// Every option of a choice control as `(value, label, description)`.
fn annotated_options(
    form: &schemaform::Form,
    identity: InstanceIdentity,
) -> Vec<(Value, String, Option<String>)> {
    form.node(identity)
        .expect("the choice control should exist")
        .definition()
        .choice_options()
        .map(|option| {
            (
                option.value().clone(),
                option.label().to_owned(),
                option.description().map(str::to_owned),
            )
        })
        .collect()
}

/// Compiles `schema` strictly and leniently, asserting neither entry point reports any
/// capability finding, and returns the strict definition.
///
/// The root object is closed first so the only findings a recognised constant choice could
/// contribute are its own; an open root would otherwise report the unrelated, non-blocking
/// `applicator.additional-properties.open`.
fn compile_without_findings(mut schema: Value) -> FormDefinition {
    schema["additionalProperties"] = json!(false);
    let definition = FormDefinition::compiler(schema.clone())
        .compile()
        .expect("the constant choice should compile strictly");
    assert_eq!(
        definition.capability_findings().count(),
        0,
        "a recognised constant choice emits no capability finding"
    );
    let analysis = FormDefinition::compiler(schema)
        .analyze()
        .expect("lenient analysis should agree");
    assert_eq!(analysis.capability_report().findings().count(), 0);
    assert_eq!(
        analysis.definition().fingerprint(),
        definition.fingerprint()
    );
    definition
}

#[test]
fn titled_one_of_and_any_of_strings_compile_to_a_labeled_choice_in_authored_order() {
    for keyword in ["oneOf", "anyOf"] {
        let definition = FormDefinition::compile(object_schema(json!({
            "title": "Priority",
            keyword: [
                { "const": "low", "title": "Low priority" },
                { "const": "medium", "title": "Medium priority" },
                { "const": "high", "title": "High priority" }
            ]
        })))
        .unwrap_or_else(|error| {
            panic!("a titled {keyword} constant choice should compile: {error}")
        });
        let mut form = definition
            .create_form(json!({ "priority": "medium" }))
            .expect("the constant choice form should be created");
        let priority = control_with_binding(&form, "/priority");
        let node = form
            .node(priority)
            .expect("the choice control should exist");

        assert_eq!(
            node.definition().semantic_kind(),
            Some(SemanticKind::Choice),
            "{keyword} constant choices are ordinary choice controls"
        );
        assert!(node.definition().is_choice_selectable());
        assert_eq!(
            options(&form, priority),
            [
                (json!("low"), "Low priority".to_owned()),
                (json!("medium"), "Medium priority".to_owned()),
                (json!("high"), "High priority".to_owned()),
            ],
            "{keyword} options keep the authored branch order and titles"
        );
        assert_eq!(
            node.selected_choice().map(|option| option.label()),
            Some("Medium priority")
        );
        assert_eq!(node.display_text().as_deref(), Some("Medium priority"));
        assert!(node.allowed_operations().can_set_value());
        assert!(!node.is_dirty());

        form.user()
            .set_value(priority, json!("high"))
            .expect("a constant option should be selectable");
        assert_eq!(form.form_data(), &json!({ "priority": "high" }));
        assert!(
            form.node(priority)
                .expect("the choice should remain")
                .is_dirty()
        );
        let snapshot = match form.prepare_submission().outcome() {
            SubmissionOutcome::Ready(snapshot) => snapshot.clone(),
            SubmissionOutcome::Blocked(_) => {
                panic!("the selected constant should be submittable")
            }
        };
        assert_eq!(snapshot.form_data(), &json!({ "priority": "high" }));
    }
}

#[test]
fn disqualified_branch_shapes_keep_the_blocking_finding_and_name_the_reason() {
    for (keyword, code, branches, reason) in [
        (
            "oneOf",
            "applicator.one-of",
            json!([
                { "const": "a", "title": "First" },
                { "const": "a", "title": "Same constant again" }
            ]),
            "duplicate-constant",
        ),
        (
            "oneOf",
            "applicator.one-of",
            json!([{ "const": "a", "minLength": 1 }, { "const": "b" }]),
            "non-constant-branch",
        ),
        (
            "anyOf",
            "applicator.any-of",
            json!([{ "const": "a" }, { "enum": ["b", "c"], "title": "Carrier" }]),
            "non-constant-branch",
        ),
        (
            "oneOf",
            "applicator.one-of",
            json!([{ "const": "a", "title": "Text" }, true]),
            "boolean-branch",
        ),
        (
            "anyOf",
            "applicator.any-of",
            json!([{ "const": "a" }, { "$ref": "#/$defs/other" }]),
            "non-constant-branch",
        ),
    ] {
        let schema = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$defs": { "other": { "const": "b" } },
            "type": "object",
            "properties": { "value": { keyword: branches } }
        });
        let expected_parameters = json!({ "branchCount": 2, "reason": reason });

        let report = match FormDefinition::compiler(schema.clone()).compile() {
            Err(CompileError::Capability(report)) => report,
            Err(error) => panic!("{keyword} {reason}: unexpected error {error}"),
            Ok(_) => panic!("{keyword} {reason}: a disqualified applicator must not compile"),
        };
        let blocking = report
            .findings()
            .filter(|finding| finding.is_blocking())
            .collect::<Vec<_>>();
        assert_eq!(blocking.len(), 1, "{keyword} {reason}: exactly one blocker");
        assert_eq!(blocking[0].code(), code);
        assert_eq!(blocking[0].instance_location().as_str(), "/value");
        assert_eq!(
            blocking[0].keyword_location().pointer().as_str(),
            format!("/properties/value/{keyword}")
        );
        assert_eq!(blocking[0].parameters(), &expected_parameters);

        let analysis = FormDefinition::compiler(schema)
            .analyze()
            .expect("lenient analysis keeps the disqualified applicator as an unsupported region");
        assert_eq!(analysis.capability_report(), &report);
        let children = analysis
            .definition()
            .node(analysis.definition().root())
            .expect("the root should exist")
            .children()
            .collect::<Vec<_>>();
        assert_eq!(
            analysis
                .definition()
                .node(children[0])
                .expect("the value node should exist")
                .kind(),
            DefinitionNodeKind::Unsupported
        );
    }
}

#[test]
fn mixed_scalar_constants_round_trip_with_type_filtering_title_fallback_and_descriptions() {
    let definition = FormDefinition::compile(object_schema(json!({
        "title": "Choice",
        "oneOf": [
            { "const": null, "title": "Not specified", "description": "Leave the priority open" },
            { "const": 1, "type": "integer", "title": "One" },
            { "const": true, "type": "boolean" },
            { "const": "eu", "type": ["string", "null"], "title": "Europe", "description": "EU region" },
            { "const": "mismatched", "type": "integer", "title": "Never validates" },
            { "const": 2.5, "type": "integer", "title": "Not an integer either" }
        ]
    })))
    .expect("mixed annotated constants should compile");
    let mut form = definition
        .create_form(json!({ "priority": 1 }))
        .expect("the mixed choice form should be created");
    let priority = control_with_binding(&form, "/priority");
    let node = form
        .node(priority)
        .expect("the choice control should exist");

    assert_eq!(
        node.definition().semantic_kind(),
        Some(SemanticKind::Choice)
    );
    assert_eq!(
        annotated_options(&form, priority),
        [
            (
                Value::Null,
                "Not specified".to_owned(),
                Some("Leave the priority open".to_owned()),
            ),
            (json!(1), "One".to_owned(), None),
            (json!(true), "true".to_owned(), None),
            (
                json!("eu"),
                "Europe".to_owned(),
                Some("EU region".to_owned())
            ),
        ],
        "inconsistent types are filtered out, a missing title falls back to the value"
    );
    assert!(node.definition().accepts_null());

    for value in [json!(true), json!("eu"), json!(1)] {
        form.user()
            .set_value(priority, value.clone())
            .expect("every surviving constant should be selectable");
        assert_eq!(form.form_data(), &json!({ "priority": value }));
        assert_eq!(
            form.view().validation_outcome(),
            ValidationOutcomeView::Valid
        );
    }
    form.user()
        .set_null(priority)
        .expect("the titled null constant is an ordinary option");
    assert_eq!(form.form_data(), &json!({ "priority": null }));
    assert_eq!(
        form.node(priority)
            .expect("the choice should remain")
            .display_text()
            .as_deref(),
        Some("Not specified")
    );
}

#[test]
fn any_of_merges_duplicate_constants_and_the_first_branch_annotations_win() {
    let definition = compile_without_findings(object_schema(json!({
        "anyOf": [
            { "const": "high", "title": "High priority", "description": "Urgent" },
            { "const": "low", "title": "Low priority" },
            { "const": "high", "title": "Also high", "description": "Repeated" },
            { "const": "low" }
        ]
    })));

    let mut form = definition
        .create_form(json!({ "priority": "low" }))
        .expect("the merged choice form should be created");
    let priority = control_with_binding(&form, "/priority");
    assert_eq!(
        annotated_options(&form, priority),
        [
            (
                json!("high"),
                "High priority".to_owned(),
                Some("Urgent".to_owned()),
            ),
            (json!("low"), "Low priority".to_owned(), None),
        ],
        "each value appears once, labeled by the first branch that carries it"
    );

    form.user()
        .set_value(priority, json!("high"))
        .expect("the merged constant should be selectable");
    assert_eq!(form.form_data(), &json!({ "priority": "high" }));
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid,
        "anyOf accepts a value matched by more than one branch"
    );
}

#[test]
fn differing_titles_across_recognised_applicators_take_the_first_in_canonical_order() {
    let definition = compile_without_findings(object_schema(json!({
        "allOf": [
            {
                "anyOf": [
                    { "const": "low", "title": "Lowest", "description": "Bottom of the queue" },
                    { "const": "high", "title": "Highest" }
                ]
            },
            {
                "oneOf": [
                    { "const": "high", "title": "High priority", "description": "Urgent" },
                    { "const": "low", "title": "Low priority" }
                ]
            }
        ]
    })));

    let form = definition
        .create_form(json!({ "priority": "high" }))
        .expect("the form should be created");
    let priority = control_with_binding(&form, "/priority");
    assert_eq!(
        annotated_options(&form, priority),
        [
            (
                json!("low"),
                "Lowest".to_owned(),
                Some("Bottom of the queue".to_owned()),
            ),
            (json!("high"), "Highest".to_owned(), None),
        ],
        "order, title and description follow the first applicator in canonical location order"
    );
    assert_eq!(
        form.node(priority)
            .expect("the choice should exist")
            .display_text()
            .as_deref(),
        Some("Highest")
    );
}

#[test]
fn an_option_title_is_the_authored_branch_title_and_absent_when_the_label_is_a_spelling() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "titled": {
                "oneOf": [
                    { "const": null, "title": "null" },
                    { "const": true, "title": "Yes" },
                    { "const": false }
                ]
            },
            "plain": { "enum": ["a", null] }
        }
    }))
    .expect("both spellings should compile side by side");
    let form = definition
        .create_form(json!({}))
        .expect("the form should be created");
    let titles = |binding: &str| {
        form.node(control_with_binding(&form, binding))
            .expect("the choice control should exist")
            .definition()
            .choice_options()
            .map(|option| (option.label().to_owned(), option.title().map(str::to_owned)))
            .collect::<Vec<_>>()
    };

    assert_eq!(
        titles("/titled"),
        [
            ("null".to_owned(), Some("null".to_owned())),
            ("Yes".to_owned(), Some("Yes".to_owned())),
            ("false".to_owned(), None),
        ],
        "a title reads the same as a spelling but is still an authored title"
    );
    assert_eq!(
        titles("/plain"),
        [("null".to_owned(), None), ("a".to_owned(), None)],
        "enum options are never titled, and enum still sorts its null option first"
    );
}

#[test]
fn constant_choices_keep_authored_order_while_an_equivalent_enum_stays_sorted() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "properties": {
            "titled": {
                "anyOf": [
                    { "const": "zulu", "title": "Zulu" },
                    { "const": "alpha", "title": "Alpha" },
                    { "const": "mike", "title": "Mike" }
                ]
            },
            "plain": { "enum": ["zulu", "alpha", "mike"] }
        }
    }))
    .expect("both spellings should compile side by side");
    let form = definition
        .create_form(json!({}))
        .expect("the ordering form should be created");

    assert_eq!(
        options(&form, control_with_binding(&form, "/titled"))
            .into_iter()
            .map(|(value, _)| value)
            .collect::<Vec<_>>(),
        [json!("zulu"), json!("alpha"), json!("mike")],
        "a constant choice keeps the authored branch order"
    );
    assert_eq!(
        options(&form, control_with_binding(&form, "/plain"))
            .into_iter()
            .map(|(value, _)| value)
            .collect::<Vec<_>>(),
        [json!("alpha"), json!("mike"), json!("zulu")],
        "enum ordering is unchanged"
    );
}

#[test]
fn all_of_intersections_narrow_the_option_set_and_keep_surviving_titles() {
    let definition = FormDefinition::compile(object_schema(json!({
        "allOf": [
            {
                "oneOf": [
                    { "const": "high", "title": "High priority", "description": "Urgent" },
                    { "const": "medium", "title": "Medium priority" },
                    { "const": "low", "title": "Low priority" }
                ]
            },
            { "enum": ["low", "high", "unlisted"] },
            { "type": ["string", "null"] }
        ]
    })))
    .expect("an intersected constant choice should compile");
    let form = definition
        .create_form(json!({ "priority": "low" }))
        .expect("the intersected form should be created");
    let priority = control_with_binding(&form, "/priority");
    let node = form
        .node(priority)
        .expect("the choice control should exist");

    assert_eq!(
        node.definition().semantic_kind(),
        Some(SemanticKind::Choice)
    );
    assert!(node.definition().is_choice_selectable());
    assert_eq!(
        annotated_options(&form, priority),
        [
            (
                json!("high"),
                "High priority".to_owned(),
                Some("Urgent".to_owned()),
            ),
            (json!("low"), "Low priority".to_owned(), None),
        ],
        "the intersection keeps authored order and the surviving titles"
    );
}

#[test]
fn an_applicable_const_turns_the_constant_choice_into_a_fixed_constant() {
    let definition = FormDefinition::compile(object_schema(json!({
        "allOf": [
            {
                "oneOf": [
                    { "const": "high", "title": "High priority" },
                    { "const": "low", "title": "Low priority" }
                ]
            },
            { "const": "high" }
        ]
    })))
    .expect("a constant choice narrowed to one constant should compile");
    let form = definition
        .create_form(json!({ "priority": "high" }))
        .expect("the constant form should be created");
    let priority = control_with_binding(&form, "/priority");
    let node = form
        .node(priority)
        .expect("the constant control should exist");

    assert_eq!(
        node.definition().semantic_kind(),
        Some(SemanticKind::Choice)
    );
    assert!(!node.definition().is_choice_selectable());
    assert_eq!(
        options(&form, priority),
        [(json!("high"), "High priority".to_owned())]
    );
    assert_eq!(node.display_text().as_deref(), Some("High priority"));
    assert_eq!(node.allowed_operations(), Default::default());
}

#[test]
fn an_empty_intersection_is_a_located_blocking_finding_in_strict_and_lenient_modes() {
    for (property, keyword, expected_pointer) in [
        (
            json!({
                "type": "integer",
                "oneOf": [
                    { "const": "high", "title": "High priority" },
                    { "const": "low", "title": "Low priority" }
                ]
            }),
            "oneOf",
            "/properties/priority/oneOf",
        ),
        (
            json!({
                "allOf": [
                    { "anyOf": [{ "const": "high" }, { "const": "low" }] },
                    { "enum": ["medium"] }
                ]
            }),
            "anyOf",
            "/properties/priority/allOf/0/anyOf",
        ),
        (
            json!({
                "anyOf": [{ "const": "high", "type": "integer" }]
            }),
            "anyOf",
            "/properties/priority/anyOf",
        ),
    ] {
        let schema = object_schema(property);
        let report = match FormDefinition::compiler(schema.clone()).compile() {
            Err(CompileError::Capability(report)) => report,
            Err(error) => panic!("unexpected error {error}"),
            Ok(_) => panic!("an empty option set must not compile strictly"),
        };
        let blocking = report
            .findings()
            .filter(|finding| finding.is_blocking())
            .collect::<Vec<_>>();
        assert_eq!(blocking.len(), 1);
        assert_eq!(
            blocking[0].code(),
            "applicator.constant-choices.incompatible"
        );
        assert_eq!(blocking[0].instance_location().as_str(), "/priority");
        assert_eq!(
            blocking[0].keyword_location().pointer().as_str(),
            expected_pointer
        );
        assert_eq!(blocking[0].parameters(), &json!({ "keyword": keyword }));

        let analysis = FormDefinition::compiler(schema)
            .analyze()
            .expect("lenient analysis keeps the empty choice as an unsupported region");
        assert_eq!(analysis.capability_report(), &report);
    }
}

#[test]
fn data_outside_the_option_set_is_invalid_blocks_gated_submission_and_is_advised() {
    for keyword in ["oneOf", "anyOf"] {
        let definition = FormDefinition::compile(object_schema(json!({
            keyword: [
                { "const": "high", "title": "High priority" },
                { "const": "low", "title": "Low priority" }
            ]
        })))
        .expect("the constant choice should compile");
        let mut form = definition
            .create_form(json!({ "priority": "high" }))
            .expect("the form should be created");
        let priority = control_with_binding(&form, "/priority");

        assert_eq!(
            form.user().set_value(priority, json!("urgent")),
            Err(UserOperationError::OperationNotAllowed),
            "{keyword}: a value outside the option set is not a user operation"
        );
        assert_eq!(form.form_data(), &json!({ "priority": "high" }));

        let pointer = JsonPointer::parse("/priority").expect("the pointer should be valid");
        form.transact(|draft| draft.set(&pointer, json!("urgent")))
            .expect("the host may install data outside the option set");
        assert!(
            matches!(
                form.view().validation_outcome(),
                ValidationOutcomeView::Invalid { findings, .. }
                    if findings.iter().any(|finding| {
                        finding.code() == keyword && finding.instance_location().as_str() == "/priority"
                    })
            ),
            "{keyword}: the validator's own keyword reports form data outside the option set"
        );
        assert!(matches!(
            form.prepare_submission().outcome(),
            SubmissionOutcome::Blocked(blockers)
                if blockers.iter().any(|blocker| matches!(
                    blocker,
                    SubmissionBlocker::Validation(finding) if finding.code() == keyword
                ))
        ));
        let (_, advisory) = form.prepare_advisory_submission().into_parts();
        assert_eq!(advisory.form_data(), &json!({ "priority": "urgent" }));
        assert!(advisory.findings().any(|finding| matches!(
            finding,
            SubmissionBlocker::Validation(finding) if finding.code() == keyword
        )));
    }
}

#[test]
fn fingerprints_track_option_order_titles_and_descriptions() {
    let compile = |branches: Value| {
        FormDefinition::compile(object_schema(json!({ "oneOf": branches })))
            .expect("the constant choice should compile")
            .fingerprint()
    };
    let baseline = json!([
        { "const": "high", "title": "High priority", "description": "Urgent" },
        { "const": "low", "title": "Low priority" }
    ]);

    assert_eq!(compile(baseline.clone()), compile(baseline.clone()));
    assert_ne!(
        compile(baseline.clone()),
        compile(json!([
            { "const": "low", "title": "Low priority" },
            { "const": "high", "title": "High priority", "description": "Urgent" }
        ])),
        "branch order is observable"
    );
    assert_ne!(
        compile(baseline.clone()),
        compile(json!([
            { "const": "high", "title": "Highest priority", "description": "Urgent" },
            { "const": "low", "title": "Low priority" }
        ])),
        "a title change is observable"
    );
    assert_ne!(
        compile(baseline),
        compile(json!([
            { "const": "high", "title": "High priority", "description": "Pressing" },
            { "const": "low", "title": "Low priority" }
        ])),
        "a description change is observable"
    );
}

#[test]
fn titled_any_of_items_compile_to_a_homogeneous_array_of_labeled_choices() {
    let definition = FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "required": ["regions"],
        "properties": {
            "regions": {
                "title": "Regions",
                "type": "array",
                "minItems": 1,
                "maxItems": 2,
                "items": {
                    "anyOf": [
                        { "const": null, "title": "Anywhere" },
                        { "const": "eu", "title": "Europe", "description": "EU member states" },
                        { "const": "us", "title": "United States" }
                    ]
                }
            }
        }
    }))
    .expect("titled anyOf items should compile");
    let mut form = definition
        .create_form(json!({ "regions": ["us"] }))
        .expect("the array form should be created");
    let regions = control_with_binding(&form, "/regions");
    let array = form.node(regions).expect("the array should exist");
    assert_eq!(
        array.definition().semantic_kind(),
        Some(SemanticKind::HomogeneousArray)
    );
    let items = array.children().collect::<Vec<_>>();
    assert_eq!(items.len(), 1);
    let item = form.node(items[0]).expect("the item should exist");
    assert_eq!(
        item.definition().semantic_kind(),
        Some(SemanticKind::Choice)
    );
    assert!(item.definition().is_choice_selectable());
    assert_eq!(
        annotated_options(&form, items[0]),
        [
            (Value::Null, "Anywhere".to_owned(), None),
            (
                json!("eu"),
                "Europe".to_owned(),
                Some("EU member states".to_owned()),
            ),
            (json!("us"), "United States".to_owned(), None),
        ]
    );
    assert_eq!(item.display_text().as_deref(), Some("United States"));
    assert!(array.allowed_operations().can_append_item());
    assert!(
        !array.allowed_operations().can_remove_item(),
        "minItems gates removal of the only item"
    );

    form.user()
        .append_item(regions)
        .expect("append should seed the first authored non-null option");
    assert_eq!(form.form_data(), &json!({ "regions": ["us", "eu"] }));
    let array = form.node(regions).expect("the array should remain");
    assert!(
        !array.allowed_operations().can_append_item(),
        "maxItems gates append"
    );
    assert!(array.allowed_operations().can_remove_item());
    let appended = array
        .children()
        .last()
        .expect("the appended item should exist");
    form.user()
        .set_null(appended)
        .expect("the titled null option is selectable on an item");
    assert_eq!(form.form_data(), &json!({ "regions": ["us", null] }));
    assert_eq!(
        form.view().validation_outcome(),
        ValidationOutcomeView::Valid
    );
    let snapshot = match form.prepare_submission().outcome() {
        SubmissionOutcome::Ready(snapshot) => snapshot.clone(),
        SubmissionOutcome::Blocked(_) => panic!("the labeled items should be submittable"),
    };
    assert_eq!(snapshot.form_data(), &json!({ "regions": ["us", null] }));
}

#[test]
fn a_recognised_choice_reports_no_capability_finding_in_either_mode() {
    compile_without_findings(object_schema(json!({
        "oneOf": [
            { "const": "high", "title": "High priority", "x-icon": "flame", "$comment": "hot" },
            { "const": "low", "title": "Low priority", "deprecated": true, "examples": ["low"] }
        ]
    })));
}

#[test]
fn an_unrecognised_applicator_beside_a_recognised_one_still_blocks() {
    let report = match FormDefinition::compiler(object_schema(json!({
        "oneOf": [
            { "const": "high", "title": "High priority" },
            { "const": "low", "title": "Low priority" }
        ],
        "anyOf": [{ "type": "string" }, { "type": "integer" }]
    })))
    .compile()
    {
        Err(CompileError::Capability(report)) => report,
        Err(error) => panic!("unexpected error {error}"),
        Ok(_) => panic!("the general anyOf must keep blocking"),
    };
    let blocking = report
        .findings()
        .filter(|finding| finding.is_blocking())
        .map(|finding| (finding.code(), finding.parameters().clone()))
        .collect::<Vec<_>>();
    assert_eq!(
        blocking,
        [(
            "applicator.any-of",
            json!({ "branchCount": 2, "reason": "non-constant-branch" }),
        )]
    );
}

fn control_with_binding(form: &schemaform::Form, binding: &str) -> InstanceIdentity {
    let mut pending = vec![form.view().root()];
    while let Some(identity) = pending.pop() {
        let node = form.node(identity).expect("the form node should exist");
        if node
            .binding()
            .is_some_and(|current| current.pointer().as_str() == binding)
        {
            return identity;
        }
        pending.extend(node.children());
    }
    panic!("the bound control should exist")
}
