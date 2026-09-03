use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use serde_json::{Map, Value, json};

const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";
const DRAFT_KEYWORDS: &[&str] = &[
    "$schema",
    "$vocabulary",
    "$id",
    "$anchor",
    "$dynamicAnchor",
    "$dynamicRef",
    "$defs",
    "$ref",
    "$comment",
    "type",
    "enum",
    "const",
    "multipleOf",
    "maximum",
    "exclusiveMaximum",
    "minimum",
    "exclusiveMinimum",
    "maxLength",
    "minLength",
    "pattern",
    "maxItems",
    "minItems",
    "uniqueItems",
    "maxContains",
    "minContains",
    "contains",
    "maxProperties",
    "minProperties",
    "required",
    "dependentRequired",
    "properties",
    "additionalProperties",
    "patternProperties",
    "propertyNames",
    "dependentSchemas",
    "items",
    "prefixItems",
    "allOf",
    "anyOf",
    "oneOf",
    "not",
    "if",
    "then",
    "else",
    "unevaluatedItems",
    "unevaluatedProperties",
    "title",
    "description",
    "default",
    "deprecated",
    "readOnly",
    "writeOnly",
    "examples",
    "format",
    "contentEncoding",
    "contentMediaType",
    "contentSchema",
];

#[test]
fn business_schema_corpus_contains_an_attributed_draft_2020_12_fixture() {
    let corpus_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("testing/fixtures/business-schemas");
    let manifest = parse_json(corpus_root.join("manifest.json"));
    let fixtures = manifest["fixtures"]
        .as_array()
        .expect("the corpus manifest should contain a fixtures array");

    assert!(!fixtures.is_empty(), "the corpus should not be empty");
    for fixture in fixtures {
        let id = required_string(fixture, "id");
        let attribution = fixture["attribution"]
            .as_object()
            .unwrap_or_else(|| panic!("fixture {id} should record attribution"));
        for field in [
            "publisher",
            "work",
            "source_url",
            "source_revision",
            "license_spdx",
            "license_url",
            "adaptation",
        ] {
            assert!(
                attribution
                    .get(field)
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.is_empty()),
                "fixture {id} should record attribution.{field}"
            );
        }

        let resources = fixture["resources"]
            .as_array()
            .unwrap_or_else(|| panic!("fixture {id} should declare its resources"));
        assert_eq!(
            resources
                .iter()
                .filter(|resource| resource["role"] == "root")
                .count(),
            1,
            "fixture {id} should declare exactly one root resource"
        );

        for resource in resources {
            let path = required_string(resource, "path");
            let schema = parse_json(corpus_root.join(path));
            assert_eq!(
                schema["$schema"], DRAFT_2020_12,
                "fixture {id} resource {path} should declare Draft 2020-12"
            );
        }
    }
}

#[test]
fn support_profile_is_complete_and_matches_corpus_classifications() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest =
        parse_json(repository_root.join("testing/fixtures/business-schemas/manifest.json"));
    let support_profile = parse_json(repository_root.join("testing/support-profile.json"));
    assert_eq!(support_profile["format_version"], 1);
    assert_eq!(
        support_profile["status"],
        "unpublished-first-release-candidate"
    );
    assert_eq!(support_profile["current_product"], "implemented");

    let classifications = support_profile["classifications"]
        .as_array()
        .expect("the support profile should declare its classifications")
        .iter()
        .map(|classification| {
            classification
                .as_str()
                .expect("support-profile classifications should be strings")
        })
        .collect::<HashSet<_>>();
    assert_eq!(
        classifications,
        HashSet::from([
            "editing",
            "validation-only",
            "annotation",
            "warning",
            "capability-blocking",
        ]),
        "the support profile should expose exactly the five capability outcomes"
    );
    let current_states = support_profile["current_states"]
        .as_array()
        .expect("the support profile should declare its current states")
        .iter()
        .map(|state| {
            state
                .as_str()
                .expect("support-profile current states should be strings")
        })
        .collect::<HashSet<_>>();
    assert_eq!(
        current_states,
        HashSet::from(["implemented"]),
        "the support profile should expose exactly the documented current states"
    );

    let mut qualification_ids = HashSet::new();
    for failure in support_profile["qualification_failures"]
        .as_array()
        .expect("the support profile should declare qualification failures")
    {
        let id = required_string(failure, "id");
        assert_eq!(
            required_string(failure, "target"),
            "qualification-error",
            "support-profile qualification failure {id} should remain distinct from capability outcomes"
        );
        assert!(
            current_states.contains(required_string(failure, "current")),
            "support-profile qualification failure {id} has an unknown current state"
        );
        assert!(
            !required_string(failure, "context").is_empty(),
            "support-profile qualification failure {id} should explain its context"
        );
        assert!(
            qualification_ids.insert(id),
            "support-profile qualification failure {id} is duplicated"
        );
    }
    assert_eq!(
        qualification_ids,
        HashSet::from([
            "qualification.data-schema.invalid-json",
            "qualification.data-schema.dialect",
            "qualification.data-schema.vocabulary",
            "qualification.data-schema.meta-schema",
            "qualification.data-schema.resource-identity",
            "qualification.data-schema.reference",
            "qualification.ui-schema.invalid-document",
        ]),
        "the support profile should retain the complete qualification-failure boundary"
    );

    let mut profile_cases = HashMap::new();
    let mut represented_classifications = HashSet::new();
    for profile_case in support_profile["cases"]
        .as_array()
        .expect("the support profile should contain cases")
    {
        let profile_id = required_string(profile_case, "id").to_owned();
        let target = required_string(profile_case, "target");
        assert!(
            classifications.contains(target),
            "support-profile case {profile_id} has an unknown target classification"
        );
        represented_classifications.insert(target);
        let current = required_string(profile_case, "current");
        assert!(
            current_states.contains(current),
            "support-profile case {profile_id} has an unknown current state"
        );
        assert!(
            !required_string(profile_case, "context").is_empty(),
            "support-profile case {profile_id} should explain its context"
        );
        assert!(
            profile_cases
                .insert(
                    profile_id.clone(),
                    (
                        required_string(profile_case, "construct").to_owned(),
                        target.to_owned(),
                        current.to_owned(),
                    ),
                )
                .is_none(),
            "support-profile case {profile_id} is duplicated"
        );
    }
    assert!(
        !profile_cases.is_empty(),
        "the support profile should expose executable classification cases"
    );
    assert_eq!(
        represented_classifications, classifications,
        "every capability outcome should have at least one contextual case"
    );
    assert!(
        profile_cases
            .keys()
            .all(|id| !qualification_ids.contains(id.as_str())),
        "capability and qualification IDs should be disjoint"
    );

    let represented_constructs = profile_cases
        .values()
        .map(|(construct, _, _)| construct.as_str())
        .collect::<HashSet<_>>();
    for construct in DRAFT_KEYWORDS {
        assert!(
            represented_constructs.contains(construct),
            "the support profile should classify Draft 2020-12 construct {construct}"
        );
    }
    for (profile_id, expected_target, expected_current) in [
        ("structure.root.fixed-object", "editing", "implemented"),
        (
            "structure.root.scalar",
            "capability-blocking",
            "implemented",
        ),
        ("structure.root.array", "capability-blocking", "implemented"),
        ("structure.object.nested-fixed", "editing", "implemented"),
        (
            "structure.object.array-item-nested-fixed",
            "editing",
            "implemented",
        ),
        (
            "structure.object.nullable-fixed",
            "capability-blocking",
            "implemented",
        ),
        ("structure.scalar.string", "editing", "implemented"),
        ("structure.scalar.number", "editing", "implemented"),
        ("structure.scalar.integer", "editing", "implemented"),
        ("structure.scalar.boolean", "editing", "implemented"),
        ("structure.scalar.null-only", "editing", "implemented"),
        ("structure.scalar.nullable", "editing", "implemented"),
        (
            "structure.array.homogeneous-scalar",
            "editing",
            "implemented",
        ),
        (
            "structure.array.homogeneous-fixed-object",
            "editing",
            "implemented",
        ),
        (
            "structure.array.nullable",
            "capability-blocking",
            "implemented",
        ),
        (
            "structure.array.nested",
            "capability-blocking",
            "implemented",
        ),
        (
            "structure.recursive.validation",
            "validation-only",
            "implemented",
        ),
        (
            "structure.recursive.projection",
            "capability-blocking",
            "implemented",
        ),
        (
            "applicator.additional-properties.open",
            "warning",
            "implemented",
        ),
        (
            "applicator.additional-properties.schema-projection",
            "warning",
            "implemented",
        ),
        (
            "applicator.additional-properties.dynamic-map",
            "capability-blocking",
            "implemented",
        ),
        (
            "applicator.pattern-properties.fixed-projection",
            "warning",
            "implemented",
        ),
        (
            "applicator.pattern-properties.shape",
            "capability-blocking",
            "implemented",
        ),
        ("validation.unique-items", "validation-only", "implemented"),
        ("validation.contains", "validation-only", "implemented"),
        ("validation.min-contains", "validation-only", "implemented"),
        ("validation.max-contains", "validation-only", "implemented"),
        ("ui-schema.document.v1", "editing", "implemented"),
        ("ui-schema.document.omitted", "editing", "implemented"),
        ("ui-schema.element.control", "editing", "implemented"),
        ("ui-schema.element.auto", "editing", "implemented"),
        ("ui-schema.element.stack", "editing", "implemented"),
        ("ui-schema.element.grid", "editing", "implemented"),
        ("ui-schema.element.group", "editing", "implemented"),
        ("ui-schema.element.tabs", "editing", "implemented"),
        ("ui-schema.element.text", "annotation", "implemented"),
        ("ui-schema.binding.root", "editing", "implemented"),
        ("ui-schema.binding.template", "editing", "implemented"),
        ("ui-schema.item-template.inline", "editing", "implemented"),
        ("ui-schema.widget.available", "editing", "implemented"),
        (
            "ui-schema.widget.unavailable",
            "capability-blocking",
            "implemented",
        ),
        ("ui-schema.extension.required", "editing", "implemented"),
        (
            "ui-schema.extension.unavailable",
            "capability-blocking",
            "implemented",
        ),
        ("adapter.render-configuration", "editing", "implemented"),
        (
            "adapter.renderer-matcher.ambiguous",
            "capability-blocking",
            "implemented",
        ),
        ("adapter.custom-control", "editing", "implemented"),
        ("adapter.layout-renderers", "editing", "implemented"),
        ("adapter.finding-presenters", "editing", "implemented"),
        ("adapter.extension-order", "editing", "implemented"),
        ("adapter.ime-composition", "editing", "implemented"),
        ("adapter.browser-csr", "editing", "implemented"),
    ] {
        let (_, actual_target, actual_current) =
            profile_cases.get(profile_id).unwrap_or_else(|| {
                panic!("the support profile should contain first-release closure case {profile_id}")
            });
        assert_eq!(
            actual_target, expected_target,
            "support-profile case {profile_id} target classification drifted"
        );
        assert_eq!(
            actual_current, expected_current,
            "support-profile case {profile_id} current state drifted"
        );
    }

    for fixture in manifest["fixtures"]
        .as_array()
        .expect("the corpus manifest should contain fixtures")
    {
        let id = required_string(fixture, "id");
        let constructs = fixture["constructs"]
            .as_array()
            .unwrap_or_else(|| panic!("fixture {id} should classify its constructs"));
        assert!(
            !constructs.is_empty(),
            "fixture {id} should classify at least one construct"
        );
        for construct in constructs {
            let profile_id = required_string(construct, "profile_id");
            let (expected_construct, expected_classification, _) =
                profile_cases.get(profile_id).unwrap_or_else(|| {
                    panic!("fixture {id} references unknown support-profile case {profile_id}")
                });
            assert_eq!(
                required_string(construct, "construct"),
                expected_construct,
                "fixture {id} construct should match support-profile case {profile_id}"
            );
            assert_eq!(
                required_string(construct, "classification"),
                expected_classification,
                "fixture {id} classification should match support-profile case {profile_id}"
            );
        }
    }
}

#[test]
fn support_profile_evidence_is_complete_and_resolvable() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let support_profile = parse_json(repository_root.join("testing/support-profile.json"));
    let evidence = support_profile["evidence"]
        .as_object()
        .expect("the support profile should contain an evidence contract");
    let targets = evidence["test_targets"]
        .as_array()
        .expect("the evidence contract should declare test targets");
    let mut test_references = HashSet::new();
    for target in targets {
        let id = required_string(target, "id");
        let path = required_string(target, "path");
        let platforms = target["platforms"]
            .as_array()
            .unwrap_or_else(|| panic!("evidence target {id} should declare platforms"));
        assert!(
            !platforms.is_empty(),
            "evidence target {id} should run somewhere"
        );
        let source = fs::read_to_string(repository_root.join(path))
            .unwrap_or_else(|error| panic!("failed to read evidence target {path}: {error}"));
        for test in target["tests"]
            .as_array()
            .unwrap_or_else(|| panic!("evidence target {id} should declare tests"))
        {
            let test = test
                .as_str()
                .unwrap_or_else(|| panic!("evidence target {id} tests should be strings"));
            assert!(
                source.contains(&format!("fn {test}(")),
                "evidence target {id} does not contain test {test}"
            );
            assert!(
                test_references.insert(format!("{id}::{test}")),
                "evidence test {id}::{test} is duplicated"
            );
        }
    }

    let cases = support_profile["cases"]
        .as_array()
        .expect("the support profile should contain cases");
    let expected_keywords = cases
        .iter()
        .filter(|case| {
            is_draft_keyword(required_string(case, "construct"))
                && required_string(case, "target") != "capability-blocking"
        })
        .map(|case| required_string(case, "id"))
        .collect::<HashSet<_>>();
    assert_evidence_case_closure(
        evidence,
        "accepted_keywords",
        expected_keywords,
        &test_references,
    );

    let expected_structures = cases
        .iter()
        .filter(|case| required_string(case, "id").starts_with("structure."))
        .map(|case| required_string(case, "id"))
        .collect::<HashSet<_>>();
    assert_evidence_case_closure(
        evidence,
        "structures",
        expected_structures,
        &test_references,
    );

    for (section, prefix) in [("ui_schema", "ui-schema."), ("adapter", "adapter.")] {
        let expected = cases
            .iter()
            .filter(|case| required_string(case, "id").starts_with(prefix))
            .map(|case| required_string(case, "id"))
            .collect::<HashSet<_>>();
        assert_evidence_case_closure(evidence, section, expected, &test_references);
    }

    for (section, expected) in [
        (
            "scalar_states",
            &["missing", "null", "empty", "compatible", "incompatible"][..],
        ),
        (
            "commands",
            &[
                "user.input-text",
                "user.blur",
                "user.set-value",
                "user.set-null",
                "user.remove-value",
                "user.replace-value",
                "user.materialize",
                "user.append-item",
                "user.insert-item-before",
                "user.remove-item",
                "user.move-item-up",
                "user.move-item-down",
                "host.set",
                "host.remove",
                "host.replace-all",
                "host.append-item",
                "host.insert-item-before",
                "host.remove-item",
                "host.move-item-up",
                "host.move-item-down",
            ],
        ),
        (
            "finding_families",
            &[
                "validation",
                "validation-findings-truncated",
                "indeterminate",
                "capability",
                "external",
                "parse",
            ],
        ),
        (
            "lifecycle_operations",
            &[
                "reset",
                "reinitialize",
                "host-transaction",
                "replace-external-findings",
                "set-finding-visibility",
            ],
        ),
        ("submission_outcomes", &["ready", "blocked"]),
    ] {
        assert_named_evidence_closure(evidence, section, expected, &test_references);
    }

    let corpus = evidence["business_schema_corpus"]
        .as_object()
        .expect("the evidence contract should declare business-schema product paths");
    assert_eq!(
        required_string(&Value::Object(corpus.clone()), "manifest"),
        "testing/fixtures/business-schemas/manifest.json"
    );
    for field in ["core_test", "browser_test"] {
        assert!(
            test_references.contains(required_string(&Value::Object(corpus.clone()), field)),
            "business-schema {field} should reference a declared test"
        );
    }
}

fn assert_evidence_case_closure<'a>(
    evidence: &'a Map<String, Value>,
    section: &str,
    expected: HashSet<&'a str>,
    tests: &HashSet<String>,
) {
    let entries = evidence[section]
        .as_array()
        .unwrap_or_else(|| panic!("evidence section {section} should be an array"));
    let mut actual = HashSet::new();
    for entry in entries {
        assert_evidence_tests(entry, section, tests);
        for profile_case in entry["profile_cases"]
            .as_array()
            .unwrap_or_else(|| panic!("evidence section {section} should reference profile cases"))
        {
            let profile_case = profile_case.as_str().unwrap_or_else(|| {
                panic!("evidence section {section} profile cases should be strings")
            });
            assert!(
                actual.insert(profile_case),
                "evidence section {section} repeats profile case {profile_case}"
            );
        }
    }
    assert_eq!(actual, expected, "evidence section {section} is incomplete");
}

fn assert_named_evidence_closure(
    evidence: &Map<String, Value>,
    section: &str,
    expected: &[&str],
    tests: &HashSet<String>,
) {
    let entries = evidence[section]
        .as_array()
        .unwrap_or_else(|| panic!("evidence section {section} should be an array"));
    let mut actual = HashSet::new();
    for entry in entries {
        let id = required_string(entry, "id");
        assert!(actual.insert(id), "evidence section {section} repeats {id}");
        assert_evidence_tests(entry, section, tests);
    }
    assert_eq!(
        actual,
        expected.iter().copied().collect(),
        "evidence section {section} is incomplete"
    );
}

fn assert_evidence_tests(entry: &Value, section: &str, tests: &HashSet<String>) {
    let references = entry["tests"]
        .as_array()
        .unwrap_or_else(|| panic!("evidence section {section} entries should cite tests"));
    assert!(
        !references.is_empty(),
        "evidence section {section} entries should cite at least one test"
    );
    for reference in references {
        let reference = reference.as_str().unwrap_or_else(|| {
            panic!("evidence section {section} test references should be strings")
        });
        assert!(
            tests.contains(reference),
            "evidence section {section} references undeclared test {reference}"
        );
    }
}

fn is_draft_keyword(construct: &str) -> bool {
    DRAFT_KEYWORDS.contains(&construct)
}

#[test]
fn every_exercised_schema_keyword_has_an_explicit_classification() {
    let corpus_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("testing/fixtures/business-schemas");
    let manifest = parse_json(corpus_root.join("manifest.json"));

    for fixture in manifest["fixtures"]
        .as_array()
        .expect("the corpus manifest should contain fixtures")
    {
        let id = required_string(fixture, "id");
        let constructs = fixture["constructs"]
            .as_array()
            .unwrap_or_else(|| panic!("fixture {id} should classify its constructs"));
        assert_eq!(
            constructs
                .iter()
                .map(|construct| required_string(construct, "profile_id"))
                .collect::<HashSet<_>>()
                .len(),
            constructs.len(),
            "fixture {id} should not repeat profile cases"
        );
        let classified_keywords = constructs
            .iter()
            .map(|construct| required_string(construct, "construct"))
            .collect::<HashSet<_>>();
        let mut exercised = HashMap::<String, Vec<KeywordOccurrence>>::new();

        for resource in fixture["resources"]
            .as_array()
            .unwrap_or_else(|| panic!("fixture {id} should declare resources"))
        {
            let resource_name = required_string(resource, "name");
            let schema = parse_json(corpus_root.join(required_string(resource, "path")));
            collect_schema_keywords(&schema, &format!("{resource_name}#"), &mut exercised);
        }

        let exercised_keywords = exercised.keys().map(String::as_str).collect::<HashSet<_>>();
        assert_eq!(
            classified_keywords, exercised_keywords,
            "fixture {id} should classify every exercised schema keyword and no absent keyword"
        );
        for (keyword, occurrences) in &exercised {
            let construct_cases = constructs
                .iter()
                .filter(|construct| construct["construct"] == *keyword)
                .collect::<Vec<_>>();
            for occurrence in occurrences {
                let declared_case = if construct_cases.len() == 1 {
                    construct_cases[0]
                } else {
                    let matching_cases = construct_cases
                        .iter()
                        .filter(|construct| {
                            construct["locations"].as_array().is_some_and(|locations| {
                                locations.iter().any(|location| {
                                    location.as_str() == Some(occurrence.location.as_str())
                                })
                            })
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(
                        matching_cases.len(),
                        1,
                        "fixture {id} should assign {keyword} at {} to exactly one contextual profile case",
                        occurrence.location
                    );
                    matching_cases[0]
                };
                let declared_profile = required_string(declared_case, "profile_id");
                let expected_profile =
                    if keyword == "additionalProperties" && occurrence.value == Value::Bool(true) {
                        // Open fixed projections and dynamic maps are distinguished by applicable
                        // schemas, so the production-path test is authoritative for this context.
                        declared_profile
                    } else {
                        profile_case_for_occurrence(keyword, occurrence)
                    };
                assert_eq!(
                    declared_profile, expected_profile,
                    "fixture {id} classifies {keyword} at {} with the wrong contextual profile case",
                    occurrence.location
                );
            }
        }
        for construct in constructs {
            let keyword = required_string(construct, "construct");
            let locations = construct["locations"].as_array().unwrap_or_else(|| {
                panic!("fixture {id} construct {keyword} should record locations")
            });
            assert!(
                !locations.is_empty(),
                "fixture {id} construct {keyword} should record a representative location"
            );
            for location in locations {
                let location = location.as_str().unwrap_or_else(|| {
                    panic!("fixture {id} construct locations should be strings")
                });
                assert!(
                    exercised[keyword]
                        .iter()
                        .any(|actual| actual.location == location),
                    "fixture {id} does not exercise {keyword} at {location}"
                );
            }
        }

        let vocabulary_set = required_string(fixture, "vocabulary_set");
        let expected_vocabularies = manifest["vocabulary_sets"][vocabulary_set]
            .as_array()
            .unwrap_or_else(|| {
                panic!("fixture {id} references unknown vocabulary set {vocabulary_set}")
            })
            .iter()
            .map(|value| {
                value.as_str().unwrap_or_else(|| {
                    panic!("vocabulary set {vocabulary_set} should contain strings")
                })
            })
            .collect::<HashSet<_>>();
        let actual_vocabularies = exercised
            .keys()
            .map(|keyword| keyword_vocabulary(keyword))
            .collect::<HashSet<_>>();
        assert_eq!(
            expected_vocabularies, actual_vocabularies,
            "fixture {id} vocabulary usage drifted"
        );
    }
}

#[test]
fn fixtures_record_reproducible_shape_and_expected_form_behavior() {
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let corpus_root = repository_root.join("testing/fixtures/business-schemas");
    let manifest = parse_json(corpus_root.join("manifest.json"));
    let support_profile = parse_json(repository_root.join("testing/support-profile.json"));
    let profile_targets = support_profile["cases"]
        .as_array()
        .expect("the support profile should contain cases")
        .iter()
        .map(|profile_case| {
            (
                required_string(profile_case, "id"),
                required_string(profile_case, "target"),
            )
        })
        .collect::<HashMap<_, _>>();

    for fixture in manifest["fixtures"]
        .as_array()
        .expect("the corpus manifest should contain fixtures")
    {
        let id = required_string(fixture, "id");
        assert!(!required_string(fixture, "title").is_empty());
        assert!(!required_string(fixture, "domain").is_empty());
        assert_eq!(required_string(fixture, "dialect"), DRAFT_2020_12);

        let mut shape = SchemaShape::default();
        let mut root_schema = None;
        let mut root_retrieval_uri = None;
        let mut resource_schemas = HashMap::new();
        let mut resource_aliases = HashMap::new();
        let mut resource_names = HashMap::new();
        let mut exercised = HashMap::<String, Vec<KeywordOccurrence>>::new();
        for resource in fixture["resources"]
            .as_array()
            .unwrap_or_else(|| panic!("fixture {id} should declare resources"))
        {
            let name = required_string(resource, "name");
            let retrieval_uri = required_string(resource, "retrieval_uri");
            let schema = parse_json(corpus_root.join(required_string(resource, "path")));
            analyze_schema(&schema, &format!("{name}#"), 0, &mut shape);
            collect_schema_keywords(&schema, &format!("{name}#"), &mut exercised);
            assert!(
                resource_schemas
                    .insert(retrieval_uri.to_owned(), schema.clone())
                    .is_none(),
                "fixture {id} should not repeat retrieval URI {retrieval_uri}"
            );
            assert!(
                resource_aliases
                    .insert(retrieval_uri.to_owned(), retrieval_uri.to_owned())
                    .is_none(),
                "fixture {id} should not repeat resource alias {retrieval_uri}"
            );
            assert!(
                resource_names
                    .insert(retrieval_uri.to_owned(), name.to_owned())
                    .is_none(),
                "fixture {id} should not repeat resource name lookup {retrieval_uri}"
            );
            if let Some(canonical_id) = schema.get("$id").and_then(Value::as_str)
                && canonical_id != retrieval_uri
            {
                assert!(
                    resource_aliases
                        .insert(canonical_id.to_owned(), retrieval_uri.to_owned())
                        .is_none(),
                    "fixture {id} should not repeat resource alias {canonical_id}"
                );
            }
            if resource["role"] == "root" {
                root_schema = Some(schema);
                root_retrieval_uri = Some(retrieval_uri.to_owned());
            }
        }
        let root_schema =
            root_schema.unwrap_or_else(|| panic!("fixture {id} should have a root schema"));
        let root_retrieval_uri = root_retrieval_uri
            .unwrap_or_else(|| panic!("fixture {id} should have a root retrieval URI"));
        let resource_graph = ResourceGraph {
            resources: &resource_schemas,
            aliases: &resource_aliases,
            resource_names: &resource_names,
        };
        let nested_array_locations = nested_array_locations(&root_retrieval_uri, &resource_graph);
        let registry = resource_schemas
            .iter()
            .try_fold(jsonschema::Registry::new(), |registry, (uri, schema)| {
                registry.add(uri, schema.clone())
            })
            .and_then(|builder| builder.prepare())
            .unwrap_or_else(|error| {
                panic!("fixture {id} resources should form a valid Draft 2020-12 registry: {error}")
            });
        jsonschema::draft202012::options()
            .with_registry(&registry)
            .build(&root_schema)
            .unwrap_or_else(|error| {
                panic!(
                    "fixture {id} references should resolve within its declared resources: {error}"
                )
            });
        shape
            .arrays
            .sort_by(|left, right| left["location"].as_str().cmp(&right["location"].as_str()));
        shape.optional_properties.sort();
        shape
            .nullable
            .sort_by(|left, right| left["location"].as_str().cmp(&right["location"].as_str()));
        shape
            .composition
            .sort_by(|left, right| left["location"].as_str().cmp(&right["location"].as_str()));
        shape
            .references
            .sort_by(|left, right| left["location"].as_str().cmp(&right["location"].as_str()));

        let expected_shape = &fixture["structure"];
        assert_eq!(
            expected_shape["max_schema_depth"].as_u64(),
            Some(shape.max_depth as u64),
            "fixture {id} should record its maximum lexical schema depth"
        );
        assert_eq!(
            expected_shape["property_count"].as_u64(),
            Some(shape.property_count as u64),
            "fixture {id} should record its declared property count"
        );
        assert_eq!(
            expected_shape["arrays"],
            Value::Array(shape.arrays.clone()),
            "fixture {id} array shape drifted"
        );
        assert_eq!(
            expected_shape["optional_properties"],
            Value::Array(
                shape
                    .optional_properties
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect()
            ),
            "fixture {id} optional-property inventory drifted"
        );
        assert_eq!(
            expected_shape["nullable"],
            Value::Array(shape.nullable.clone()),
            "fixture {id} nullable patterns drifted"
        );
        assert_eq!(
            expected_shape["composition"],
            Value::Array(shape.composition.clone()),
            "fixture {id} composition inventory drifted"
        );
        assert_eq!(
            expected_shape["references"],
            Value::Array(shape.references.clone()),
            "fixture {id} reference inventory drifted"
        );

        let expected = &fixture["expected"];
        let constructs = fixture["constructs"]
            .as_array()
            .expect("fixture constructs should be an array");
        assert_eq!(
            required_string(expected, "validation"),
            "qualified-draft-2020-12-required",
            "fixture {id} should retain the validation contract"
        );
        let controls = expected["controls"]
            .as_array()
            .filter(|values| !values.is_empty())
            .unwrap_or_else(|| panic!("fixture {id} should record expected semantic controls"));
        for control in controls {
            let binding = required_string(control, "binding");
            let bound_schema = schema_at_binding(&resource_graph, &root_retrieval_uri, binding)
                .unwrap_or_else(|| {
                    panic!("fixture {id} control binding {binding} does not resolve in its root data schema")
                });
            assert_eq!(
                required_string(control, "kind"),
                semantic_control_kind(bound_schema),
                "fixture {id} control {binding} kind disagrees with its bound data schema"
            );
        }

        let layouts = expected["layouts"]
            .as_array()
            .filter(|values| !values.is_empty())
            .unwrap_or_else(|| panic!("fixture {id} should record expected semantic layouts"));
        let layout_kinds = HashSet::from([
            "choice-dependent-object",
            "composed-fixed-object",
            "fixed-object",
            "homogeneous-array",
            "nested-object",
            "unsupported",
        ]);
        for layout in layouts {
            let layout = layout
                .as_str()
                .unwrap_or_else(|| panic!("fixture {id} layout expectations should be strings"));
            assert!(
                layout_kinds.contains(layout),
                "fixture {id} has unknown layout {layout}"
            );
            assert!(
                layout_is_supported_by_shape(
                    &root_schema,
                    &shape,
                    &nested_array_locations,
                    constructs
                        .iter()
                        .any(|construct| { construct["classification"] == "capability-blocking" }),
                    layout,
                ),
                "fixture {id} layout {layout} is not supported by its recorded schema shape"
            );
        }
        let generation = required_string(expected, "generation_alone");
        assert!(
            ["sufficient", "authored-ui-required", "capability-blocked"].contains(&generation),
            "fixture {id} has an unknown generation expectation"
        );
        let authored_ui_needs = expected["authored_ui_needs"]
            .as_array()
            .unwrap_or_else(|| panic!("fixture {id} should record authored UI needs"));
        assert!(
            generation != "authored-ui-required" || !authored_ui_needs.is_empty(),
            "fixture {id} should explain why authored UI is required"
        );
        let allowed_ui_needs = HashSet::from([
            "array-item-template",
            "grid",
            "grouping",
            "ordering",
            "tabs",
            "widget-selection",
        ]);
        for need in authored_ui_needs {
            let need = need
                .as_str()
                .unwrap_or_else(|| panic!("fixture {id} authored UI needs should be strings"));
            assert!(
                allowed_ui_needs.contains(need),
                "fixture {id} has unknown UI need {need}"
            );
        }
        let deferred_ui_needs = expected["deferred_ui_needs"].as_array();
        let allowed_deferred_ui_needs = HashSet::from(["conditional-visibility"]);
        for need in deferred_ui_needs.into_iter().flatten() {
            let need = need
                .as_str()
                .unwrap_or_else(|| panic!("fixture {id} deferred UI needs should be strings"));
            assert!(
                allowed_deferred_ui_needs.contains(need),
                "fixture {id} has unknown deferred UI need {need}"
            );
            assert!(
                !authored_ui_needs.iter().any(|authored| authored == need),
                "fixture {id} should not present deferred UI need {need} as first-release authoring"
            );
        }
        let capability_findings = expected["capability_findings"]
            .as_array()
            .unwrap_or_else(|| panic!("fixture {id} should record expected capability findings"));
        let constructs_by_profile = constructs
            .iter()
            .map(|construct| (required_string(construct, "profile_id"), construct))
            .collect::<HashMap<_, _>>();
        let mut expected_findings = exercised
            .iter()
            .flat_map(|(keyword, occurrences)| {
                occurrences.iter().filter_map(|occurrence| {
                    let profile_id =
                        profile_case_for_fixture_occurrence(constructs, keyword, occurrence);
                    let construct = constructs_by_profile
                        .get(profile_id)
                        .expect("each occurrence profile should be declared by the fixture");
                    matches!(
                        required_string(construct, "classification"),
                        "warning" | "capability-blocking"
                    )
                    .then_some((profile_id.to_owned(), occurrence.location.clone()))
                })
            })
            .collect::<HashSet<_>>();
        expected_findings.extend(
            nested_array_locations
                .iter()
                .cloned()
                .map(|location| ("structure.array.nested".to_owned(), location)),
        );
        let mut finding_keys = HashSet::new();
        for finding in capability_findings {
            let profile_id = required_string(finding, "profile_id");
            let profile_target = profile_targets.get(profile_id).unwrap_or_else(|| {
                panic!(
                    "fixture {id} capability finding references unknown profile case {profile_id}"
                )
            });
            assert_eq!(
                required_string(finding, "classification"),
                *profile_target,
                "fixture {id} finding {profile_id} classification drifted"
            );
            let schema_location = required_string(finding, "schema_location");
            required_string(finding, "instance_location");
            assert!(
                finding["parameters"].is_object(),
                "fixture {id} finding {profile_id} should record typed parameters"
            );
            assert!(
                finding_keys.insert((profile_id.to_owned(), schema_location.to_owned())),
                "fixture {id} repeats capability finding {profile_id} at {schema_location}"
            );
        }
        assert_eq!(
            finding_keys, expected_findings,
            "fixture {id} should record one finding for every warning or blocking construct occurrence"
        );
        assert_eq!(
            generation == "capability-blocked",
            expected_findings.iter().any(|(profile_id, _)| {
                profile_targets.get(profile_id.as_str()) == Some(&"capability-blocking")
            }),
            "fixture {id} generation outcome should agree with its blocking profile findings"
        );
    }
}

#[test]
fn corpus_spans_the_required_business_domains_with_twenty_public_schemas() {
    let corpus_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("testing/fixtures/business-schemas");
    let manifest = parse_json(corpus_root.join("manifest.json"));
    assert_eq!(manifest["format_version"], 1);
    let fixtures = manifest["fixtures"]
        .as_array()
        .expect("the corpus manifest should contain fixtures");
    assert!(
        fixtures.len() >= 20,
        "the corpus should contain at least 20 schemas"
    );

    let required_domains = HashSet::from([
        "onboarding",
        "billing",
        "account-settings",
        "product-configuration",
        "addresses",
        "line-items",
        "nested-preferences",
    ]);
    let domains = fixtures
        .iter()
        .map(|fixture| required_string(fixture, "domain"))
        .collect::<HashSet<_>>();
    assert!(
        required_domains.is_subset(&domains),
        "the corpus is missing required domains: {:?}",
        required_domains.difference(&domains).collect::<Vec<_>>()
    );

    let ids = fixtures
        .iter()
        .map(|fixture| required_string(fixture, "id"))
        .collect::<Vec<_>>();
    let mut sorted_ids = ids.clone();
    sorted_ids.sort_unstable();
    sorted_ids.dedup();
    assert_eq!(ids, sorted_ids, "fixture IDs should be unique and sorted");

    let fixture_directories = fs::read_dir(corpus_root.join("fixtures"))
        .expect("the fixture directory should be readable")
        .map(|entry| {
            let entry = entry.expect("fixture directory entries should be readable");
            assert!(
                entry
                    .file_type()
                    .expect("fixture file type should be readable")
                    .is_dir(),
                "the fixture root should contain only fixture directories"
            );
            entry.file_name().to_string_lossy().into_owned()
        })
        .collect::<HashSet<_>>();
    assert_eq!(
        fixture_directories,
        ids.iter().map(|id| (*id).to_owned()).collect(),
        "manifest entries and fixture directories should have exact closure"
    );

    let declared_resource_paths = fixtures
        .iter()
        .flat_map(|fixture| {
            fixture["resources"]
                .as_array()
                .expect("fixture resources should be an array")
        })
        .map(|resource| required_string(resource, "path").to_owned())
        .collect::<HashSet<_>>();
    let declared_resource_count = fixtures
        .iter()
        .map(|fixture| {
            fixture["resources"]
                .as_array()
                .expect("fixture resources should be an array")
                .len()
        })
        .sum::<usize>();
    assert_eq!(
        declared_resource_paths.len(),
        declared_resource_count,
        "fixture resource paths should be declared exactly once"
    );
    let mut actual_resource_paths = HashSet::new();
    collect_json_paths(
        &corpus_root.join("fixtures"),
        &corpus_root,
        &mut actual_resource_paths,
    );
    assert_eq!(
        declared_resource_paths, actual_resource_paths,
        "every fixture JSON document should be declared exactly once"
    );

    let mut retrieval_uris = HashSet::new();
    let mut canonical_resource_ids = HashSet::new();
    for fixture in fixtures {
        let id = required_string(fixture, "id");
        let attribution = &fixture["attribution"];
        for field in ["source_url", "license_url"] {
            assert!(
                required_string(attribution, field).starts_with("https://"),
                "fixture {id} attribution.{field} should be an HTTPS URL"
            );
        }
        assert_eq!(
            required_string(attribution, "upstream_dialect"),
            DRAFT_2020_12,
            "fixture {id} should originate as Draft 2020-12"
        );
        let retrieved_on = required_string(attribution, "retrieved_on");
        assert!(
            retrieved_on.len() == 10
                && retrieved_on.as_bytes()[4] == b'-'
                && retrieved_on.as_bytes()[7] == b'-',
            "fixture {id} should record a YYYY-MM-DD retrieval date"
        );
        if required_string(attribution, "adaptation") != "verbatim" {
            assert!(
                attribution["adaptation_notes"]
                    .as_array()
                    .is_some_and(|notes| !notes.is_empty()),
                "fixture {id} should explain its adaptation"
            );
        }

        let mut resource_names = HashSet::new();
        for resource in fixture["resources"]
            .as_array()
            .unwrap_or_else(|| panic!("fixture {id} should declare resources"))
        {
            let path = required_string(resource, "path");
            assert!(
                resource_names.insert(required_string(resource, "name")),
                "fixture {id} resource names should be unique"
            );
            assert!(
                retrieval_uris.insert(required_string(resource, "retrieval_uri")),
                "fixture {id} retrieval URIs should be unique across the corpus"
            );
            let schema = parse_json(corpus_root.join(path));
            let mut embedded_ids = Vec::new();
            collect_schema_ids(&schema, &mut embedded_ids);
            for embedded_id in embedded_ids {
                assert!(
                    canonical_resource_ids.insert(embedded_id.clone()),
                    "fixture {id} repeats canonical resource ID {embedded_id}"
                );
            }
            assert!(
                path.starts_with(&format!("fixtures/{id}/")) && !path.contains(".."),
                "fixture {id} resource path should stay inside its fixture directory"
            );
        }
    }
}

#[test]
fn property_names_do_not_create_applicator_context() {
    let schema = json!({
        "type": "object",
        "properties": {
            "anyOf": { "type": "string" },
            "if": { "type": "string" },
            "then": { "type": "string" }
        }
    });
    let mut exercised = HashMap::<String, Vec<KeywordOccurrence>>::new();

    collect_schema_keywords(&schema, "root#", &mut exercised);

    for occurrence in &exercised["type"] {
        assert_eq!(
            profile_case_for_occurrence("type", occurrence),
            "validation.type.editable"
        );
    }
}

#[test]
fn fixture_resources_are_valid_draft_2020_12_schemas() {
    let corpus_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("testing/fixtures/business-schemas");
    let manifest = parse_json(corpus_root.join("manifest.json"));

    for fixture in manifest["fixtures"]
        .as_array()
        .expect("the corpus manifest should contain fixtures")
    {
        let id = required_string(fixture, "id");
        for resource in fixture["resources"]
            .as_array()
            .unwrap_or_else(|| panic!("fixture {id} should declare resources"))
        {
            let path = required_string(resource, "path");
            let schema = parse_json(corpus_root.join(path));
            jsonschema::draft202012::meta::validate(&schema).unwrap_or_else(|error| {
                panic!("fixture {id} resource {path} is not a valid Draft 2020-12 schema: {error}")
            });
        }
    }
}

#[test]
fn resource_graph_uses_draft_resource_and_anchor_semantics() {
    let root = json!({
        "$schema": DRAFT_2020_12,
        "$id": "https://schemas.example/root.json",
        "$ref": "child.json#item",
        "title": "Reference siblings remain valid"
    });
    let child = json!({
        "$schema": DRAFT_2020_12,
        "$id": "https://schemas.example/child.json",
        "$anchor": "item",
        "type": "string"
    });
    let registry = jsonschema::Registry::new()
        .add("https://retrieval.example/root.json", root.clone())
        .and_then(|builder| builder.add("https://retrieval.example/child.json", child))
        .and_then(|builder| builder.prepare())
        .expect("retrieval and canonical resource identities should coexist");

    jsonschema::draft202012::options()
        .with_registry(&registry)
        .build(&root)
        .expect("relative references and anchors should resolve from canonical resource IDs");
}

fn parse_json(path: PathBuf) -> Value {
    let source = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    serde_json::from_str(&source)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
}

fn required_string<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} should be a string"))
}

fn collect_json_paths(directory: &Path, root: &Path, paths: &mut HashSet<String>) {
    for entry in fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
    {
        let entry = entry.expect("fixture entries should be readable");
        let file_type = entry
            .file_type()
            .expect("fixture file types should be readable");
        assert!(
            !file_type.is_symlink(),
            "fixture resources should not be symlinks"
        );
        if file_type.is_dir() {
            collect_json_paths(&entry.path(), root, paths);
        } else if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            let relative = entry
                .path()
                .strip_prefix(root)
                .expect("fixture resources should remain below the corpus root")
                .to_string_lossy()
                .replace('\\', "/");
            assert!(
                paths.insert(relative),
                "fixture resource paths should be unique"
            );
        } else {
            panic!(
                "fixture directories should contain only declared JSON resources: {}",
                entry.path().display()
            );
        }
    }
}

fn collect_schema_ids(schema: &Value, ids: &mut Vec<String>) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(id) = object.get("$id").and_then(Value::as_str) {
        ids.push(id.to_owned());
    }
    for_each_child_schema(object, "", |child, _, _| collect_schema_ids(child, ids));
}

fn collect_schema_keywords(
    schema: &Value,
    schema_location: &str,
    keywords: &mut HashMap<String, Vec<KeywordOccurrence>>,
) {
    collect_schema_keywords_in_context(schema, schema_location, SchemaContext::default(), keywords);
}

fn collect_schema_keywords_in_context(
    schema: &Value,
    schema_location: &str,
    context: SchemaContext,
    keywords: &mut HashMap<String, Vec<KeywordOccurrence>>,
) {
    let Some(object) = schema.as_object() else {
        return;
    };

    for keyword in object.keys() {
        keywords
            .entry(keyword.clone())
            .or_default()
            .push(KeywordOccurrence {
                location: format!("{schema_location}/{}", pointer_token(keyword)),
                value: object[keyword].clone(),
                context,
            });
    }

    for_each_child_schema(
        object,
        schema_location,
        |child, child_location, relation| {
            collect_schema_keywords_in_context(
                child,
                &child_location,
                context.descend(relation),
                keywords,
            );
        },
    );
}

struct KeywordOccurrence {
    location: String,
    value: Value,
    context: SchemaContext,
}

fn profile_case_for_fixture_occurrence<'a>(
    constructs: &'a [Value],
    keyword: &str,
    occurrence: &KeywordOccurrence,
) -> &'a str {
    if keyword == "additionalProperties" && occurrence.value == Value::Bool(true) {
        return constructs
            .iter()
            .filter(|construct| construct["construct"] == keyword)
            .find(|construct| {
                construct["locations"].as_array().is_some_and(|locations| {
                    locations
                        .iter()
                        .any(|location| location.as_str() == Some(occurrence.location.as_str()))
                })
            })
            .map(|construct| required_string(construct, "profile_id"))
            .expect("true additionalProperties should have a location-aware profile case");
    }
    profile_case_for_occurrence(keyword, occurrence)
}

fn profile_case_for_occurrence(keyword: &str, occurrence: &KeywordOccurrence) -> &'static str {
    match keyword {
        "$schema" => "core.schema.draft-2020-12",
        "$id" => "core.id.resource",
        "$comment" => "core.comment",
        "$defs" => "core.defs.referenced",
        "$ref" => "core.ref.resolved",
        "type" if occurrence.context.union_branch => "validation.type.union-branch",
        "type" => "validation.type.editable",
        "enum" if occurrence.context.predicate => "validation.enum.predicate",
        "enum" if occurrence.context.conditional => "validation.enum.conditional",
        "enum" => "validation.enum.static",
        "const" => panic!("the current corpus has no const profile case"),
        "required" if occurrence.context.union_branch => "validation.required.union-branch",
        "required" => "validation.required",
        "minimum" => "validation.minimum.numeric",
        "maximum" => "validation.maximum.numeric",
        "multipleOf" => "validation.multiple-of",
        "pattern" => "validation.pattern",
        "minItems" => "validation.min-items",
        "uniqueItems" => "validation.unique-items",
        "properties" if occurrence.context.predicate => "applicator.properties.predicate",
        "properties" if occurrence.context.conditional => "applicator.properties.conditional",
        "properties" if occurrence.context.union_branch => "applicator.properties.union-branch",
        "properties" => "applicator.properties.fixed",
        "items" => "applicator.items.homogeneous",
        "additionalProperties" => {
            if occurrence.context.union_branch {
                "applicator.additional-properties.union-branch"
            } else if occurrence.value == false {
                "applicator.additional-properties.closed"
            } else if occurrence.value == true {
                unreachable!("true additionalProperties requires applicable-schema context")
            } else {
                panic!("the corpus has no schema-valued additionalProperties profile case")
            }
        }
        "allOf"
            if occurrence.value.as_array().is_some_and(|branches| {
                branches.iter().any(|branch| branch.get("if").is_some())
            }) =>
        {
            "applicator.all-of.conditional"
        }
        "allOf" => "applicator.all-of.compatible",
        "anyOf" => "applicator.any-of.general",
        "if" => "applicator.if.structural",
        "then" => "applicator.then.structural",
        "format" => "format.default-annotation",
        "title" => "metadata.title",
        "description" => "metadata.description",
        "default" => "metadata.default",
        "readOnly" => "metadata.read-only",
        "writeOnly" if occurrence.context.union_branch => "metadata.write-only.union-branch",
        "writeOnly" => panic!("the corpus has no supported writeOnly profile case"),
        _ => panic!("schema keyword {keyword} has no corpus profile case mapping"),
    }
}

fn pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[derive(Clone, Copy, Default)]
struct SchemaContext {
    predicate: bool,
    conditional: bool,
    union_branch: bool,
}

impl SchemaContext {
    fn descend(self, relation: ChildRelation) -> Self {
        Self {
            predicate: self.predicate || matches!(relation, ChildRelation::Predicate),
            conditional: self.conditional || matches!(relation, ChildRelation::Conditional),
            union_branch: self.union_branch || matches!(relation, ChildRelation::UnionBranch),
        }
    }
}

#[derive(Clone, Copy)]
enum ChildRelation {
    Ordinary,
    ArrayItem,
    Predicate,
    Conditional,
    UnionBranch,
}

fn for_each_child_schema(
    object: &Map<String, Value>,
    location: &str,
    mut visit: impl FnMut(&Value, String, ChildRelation),
) {
    for keyword in [
        "$defs",
        "properties",
        "patternProperties",
        "dependentSchemas",
    ] {
        if let Some(children) = object.get(keyword).and_then(Value::as_object) {
            for (name, child) in children {
                visit(
                    child,
                    format!(
                        "{location}/{}/{}",
                        pointer_token(keyword),
                        pointer_token(name)
                    ),
                    ChildRelation::Ordinary,
                );
            }
        }
    }
    for keyword in [
        "additionalProperties",
        "unevaluatedProperties",
        "propertyNames",
        "items",
        "contains",
        "unevaluatedItems",
        "not",
        "if",
        "then",
        "else",
        "contentSchema",
    ] {
        if let Some(child) = object.get(keyword) {
            let relation = match keyword {
                "items" => ChildRelation::ArrayItem,
                "if" => ChildRelation::Predicate,
                "then" | "else" => ChildRelation::Conditional,
                _ => ChildRelation::Ordinary,
            };
            visit(
                child,
                format!("{location}/{}", pointer_token(keyword)),
                relation,
            );
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf", "prefixItems"] {
        if let Some(children) = object.get(keyword).and_then(Value::as_array) {
            for (index, child) in children.iter().enumerate() {
                visit(
                    child,
                    format!("{location}/{}/{index}", pointer_token(keyword)),
                    if keyword == "prefixItems" {
                        ChildRelation::ArrayItem
                    } else if matches!(keyword, "anyOf" | "oneOf") {
                        ChildRelation::UnionBranch
                    } else {
                        ChildRelation::Ordinary
                    },
                );
            }
        }
    }
}

fn keyword_vocabulary(keyword: &str) -> &'static str {
    match keyword {
        "$comment" | "$defs" | "$id" | "$ref" | "$schema" => {
            "https://json-schema.org/draft/2020-12/vocab/core"
        }
        "additionalProperties"
        | "allOf"
        | "anyOf"
        | "else"
        | "if"
        | "items"
        | "oneOf"
        | "properties"
        | "then" => "https://json-schema.org/draft/2020-12/vocab/applicator",
        "const" | "enum" | "maximum" | "minimum" | "minItems" | "multipleOf" | "pattern"
        | "required" | "type" | "uniqueItems" => {
            "https://json-schema.org/draft/2020-12/vocab/validation"
        }
        "default" | "description" | "readOnly" | "title" | "writeOnly" => {
            "https://json-schema.org/draft/2020-12/vocab/meta-data"
        }
        "format" => "https://json-schema.org/draft/2020-12/vocab/format-annotation",
        _ => panic!("schema keyword {keyword} has no corpus vocabulary mapping"),
    }
}

struct ResourceGraph<'a> {
    resources: &'a HashMap<String, Value>,
    aliases: &'a HashMap<String, String>,
    resource_names: &'a HashMap<String, String>,
}

struct ResolvedReference<'a, 'b> {
    schema: &'a Value,
    retrieval_uri: &'a str,
    fragment: &'b str,
}

impl<'a> ResourceGraph<'a> {
    fn resolve_reference<'b>(
        &'a self,
        current_retrieval_uri: &str,
        reference: &'b str,
    ) -> Option<ResolvedReference<'a, 'b>> {
        let (resource_uri, fragment) = reference.split_once('#').unwrap_or((reference, ""));
        let retrieval_uri = if resource_uri.is_empty() {
            self.resources
                .get_key_value(current_retrieval_uri)?
                .0
                .as_str()
        } else {
            self.aliases.get(resource_uri)?.as_str()
        };
        let document = self.resources.get(retrieval_uri)?;
        let schema = if fragment.is_empty() {
            document
        } else if fragment.starts_with('/') {
            document.pointer(fragment)?
        } else {
            return None;
        };

        Some(ResolvedReference {
            schema,
            retrieval_uri,
            fragment,
        })
    }

    fn resolve_schema(
        &'a self,
        mut retrieval_uri: &'a str,
        mut schema: &'a Value,
    ) -> Option<(&'a Value, &'a str)> {
        for _ in 0..32 {
            let Some(reference) = schema.get("$ref").and_then(Value::as_str) else {
                return Some((schema, retrieval_uri));
            };
            let resolved = self.resolve_reference(retrieval_uri, reference)?;
            schema = resolved.schema;
            retrieval_uri = resolved.retrieval_uri;
        }
        None
    }
}

fn schema_at_binding<'a>(
    graph: &'a ResourceGraph<'a>,
    root_retrieval_uri: &str,
    binding: &str,
) -> Option<&'a Value> {
    let (mut schema, mut retrieval_uri) = graph
        .resources
        .get_key_value(root_retrieval_uri)
        .map(|(uri, schema)| (schema, uri.as_str()))?;
    let tokens = binding
        .strip_prefix('/')?
        .split('/')
        .map(|token| token.replace("~1", "/").replace("~0", "~"));
    for token in tokens {
        (schema, retrieval_uri) = graph.resolve_schema(retrieval_uri, schema)?;
        let object = schema.as_object()?;
        schema = if object.get("type") == Some(&json!("array")) {
            token.parse::<usize>().ok()?;
            object.get("items")?
        } else {
            object.get("properties")?.get(&token)?
        };
    }
    graph
        .resolve_schema(retrieval_uri, schema)
        .map(|(schema, _)| schema)
}

fn nested_array_locations(root_retrieval_uri: &str, graph: &ResourceGraph<'_>) -> HashSet<String> {
    fn visit(
        schema: &Value,
        retrieval_uri: &str,
        location: &str,
        below_array_item: bool,
        graph: &ResourceGraph<'_>,
        visited: &mut HashSet<(String, bool)>,
        nested_arrays: &mut HashSet<String>,
    ) {
        if !visited.insert((location.to_owned(), below_array_item)) {
            return;
        }
        let Some(object) = schema.as_object() else {
            return;
        };

        let is_array = object.get("type") == Some(&json!("array"))
            || object
                .get("type")
                .and_then(Value::as_array)
                .is_some_and(|types| types.iter().any(|kind| kind == "array"));
        if below_array_item && is_array {
            nested_arrays.insert(location.to_owned());
        }

        if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
            let resolved = graph
                .resolve_reference(retrieval_uri, reference)
                .unwrap_or_else(|| {
                    panic!(
                        "corpus structural analysis requires JSON Pointer references: {reference}"
                    )
                });
            let target_name = graph
                .resource_names
                .get(resolved.retrieval_uri)
                .unwrap_or_else(|| {
                    panic!(
                        "corpus resource {} has no manifest name",
                        resolved.retrieval_uri
                    )
                });
            visit(
                resolved.schema,
                resolved.retrieval_uri,
                &format!("{target_name}#{}", resolved.fragment),
                below_array_item,
                graph,
                visited,
                nested_arrays,
            );
        }

        for_each_child_schema(object, location, |child, child_location, relation| {
            visit(
                child,
                retrieval_uri,
                &child_location,
                below_array_item || matches!(relation, ChildRelation::ArrayItem),
                graph,
                visited,
                nested_arrays,
            );
        });
    }

    let root = graph
        .resources
        .get(root_retrieval_uri)
        .expect("the root retrieval URI should identify a declared resource");
    let root_name = graph
        .resource_names
        .get(root_retrieval_uri)
        .expect("the root resource should have a manifest name");
    let mut visited = HashSet::new();
    let mut nested_arrays = HashSet::new();
    visit(
        root,
        root_retrieval_uri,
        &format!("{root_name}#"),
        false,
        graph,
        &mut visited,
        &mut nested_arrays,
    );

    nested_arrays
}

fn semantic_control_kind(schema: &Value) -> &'static str {
    let object = schema
        .as_object()
        .expect("a control binding should resolve to an object schema");
    if object.contains_key("anyOf") || object.contains_key("oneOf") {
        return "unsupported-union";
    }
    if object.contains_key("enum") || object.contains_key("const") {
        return "choice";
    }

    match object.get("type") {
        Some(Value::Array(types))
            if types.iter().any(|kind| kind == "string")
                && types.iter().any(|kind| kind == "null") =>
        {
            "nullable-string"
        }
        Some(Value::String(kind)) if kind == "array" => "homogeneous-array",
        Some(Value::String(kind)) if kind == "boolean" => "boolean",
        Some(Value::String(kind)) if kind == "integer" => "integer",
        Some(Value::String(kind)) if kind == "number" => "number",
        Some(Value::String(kind))
            if kind == "string" && object.get("writeOnly") == Some(&json!(true)) =>
        {
            "sensitive-string"
        }
        Some(Value::String(kind)) if kind == "string" => "string",
        _ => panic!("a control binding should resolve to a supported semantic kind"),
    }
}

fn layout_is_supported_by_shape(
    root: &Value,
    shape: &SchemaShape,
    nested_array_locations: &HashSet<String>,
    has_blocking_construct: bool,
    layout: &str,
) -> bool {
    match layout {
        "fixed-object" => root.get("type") == Some(&json!("object")),
        "nested-object" => {
            let root_property_count = root
                .get("properties")
                .and_then(Value::as_object)
                .map_or(0, Map::len);
            shape.property_count > root_property_count
        }
        "homogeneous-array" => !shape.arrays.is_empty(),
        "composed-fixed-object" => shape
            .composition
            .iter()
            .any(|entry| entry["kind"] == "all-of"),
        "choice-dependent-object" => shape
            .composition
            .iter()
            .any(|entry| entry["kind"] == "one-of"),
        "unsupported" => {
            !nested_array_locations.is_empty()
                || has_blocking_construct
                || shape
                    .composition
                    .iter()
                    .any(|entry| entry["kind"] == "any-of" || entry["kind"] == "one-of")
        }
        _ => false,
    }
}

#[derive(Default)]
struct SchemaShape {
    max_depth: usize,
    property_count: usize,
    arrays: Vec<Value>,
    optional_properties: Vec<String>,
    nullable: Vec<Value>,
    composition: Vec<Value>,
    references: Vec<Value>,
}

fn analyze_schema(schema: &Value, location: &str, depth: usize, shape: &mut SchemaShape) {
    shape.max_depth = shape.max_depth.max(depth);
    let Some(object) = schema.as_object() else {
        return;
    };

    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        shape.references.push(json!({
            "location": format!("{location}/$ref"),
            "kind": if reference.starts_with('#') { "local" } else { "supplied-resource" },
            "target": reference
        }));
    }

    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        shape.property_count += properties.len();
        let required = object
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect::<HashSet<_>>();
        for property in properties.keys() {
            if !required.contains(property.as_str()) {
                shape
                    .optional_properties
                    .push(format!("{location}/properties/{}", pointer_token(property)));
            }
        }
    }

    let types = object.get("type").and_then(Value::as_array);
    if types.is_some_and(|types| types.iter().any(|kind| kind == "null")) {
        shape
            .nullable
            .push(json!({ "location": location, "pattern": "type-union" }));
    } else if object
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|values| values.contains(&Value::Null))
    {
        shape
            .nullable
            .push(json!({ "location": location, "pattern": "enum-null" }));
    } else if object.get("const") == Some(&Value::Null) {
        shape
            .nullable
            .push(json!({ "location": location, "pattern": "const-null" }));
    } else {
        for (keyword, pattern) in [("anyOf", "any-of-null"), ("oneOf", "one-of-null")] {
            if object
                .get(keyword)
                .and_then(Value::as_array)
                .is_some_and(|branches| {
                    branches
                        .iter()
                        .any(|branch| branch.get("type") == Some(&json!("null")))
                })
            {
                shape
                    .nullable
                    .push(json!({ "location": location, "pattern": pattern }));
            }
        }
    }

    let is_array = object.get("type") == Some(&json!("array"))
        || types.is_some_and(|types| types.iter().any(|kind| kind == "array"));
    if is_array {
        let array_shape = if object.contains_key("prefixItems") {
            match object.get("items") {
                Some(Value::Bool(false)) => "tuple-closed",
                Some(Value::Object(_)) | Some(Value::Bool(true)) => "tuple-with-homogeneous-tail",
                _ => "tuple-open",
            }
        } else if object.contains_key("items") {
            "homogeneous-items"
        } else {
            "unconstrained"
        };
        shape
            .arrays
            .push(json!({ "location": location, "shape": array_shape }));
    }

    for (keyword, kind) in [
        ("allOf", "all-of"),
        ("anyOf", "any-of"),
        ("oneOf", "one-of"),
        ("not", "not"),
        ("dependentSchemas", "dependent-schemas"),
    ] {
        if object.contains_key(keyword) {
            shape.composition.push(json!({
                "location": format!("{location}/{}", pointer_token(keyword)),
                "kind": kind
            }));
        }
    }
    if ["if", "then", "else"]
        .iter()
        .any(|keyword| object.contains_key(*keyword))
    {
        shape
            .composition
            .push(json!({ "location": location, "kind": "conditional" }));
    }

    for_each_child_schema(object, location, |child, child_location, _| {
        analyze_schema(child, &child_location, depth + 1, shape);
    });
}
