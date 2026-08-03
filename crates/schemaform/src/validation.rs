use std::{cmp::Ordering, collections::HashMap};

use jsonschema::{
    Keyword, PatternOptions, ValidationError, error::ValidationErrorKind, paths::Location,
};
use num_bigint::BigInt;
use serde_json::{Value, json};

use crate::{
    JsonPointer, QualificationError, RetrievalUri, SchemaLocation, ValidationFinding,
    form::IndeterminateReason,
    resources::{DenyRetrieval, ResourceGraph},
};

#[cfg(test)]
mod official_suite;

const DEFAULT_ROOT_RETRIEVAL_URI: &str = "urn:schemaform:root";
const REGEX_SIZE_LIMIT: usize = 10 * 1024 * 1024;
const REGEX_DFA_SIZE_LIMIT: usize = 2 * 1024 * 1024;
#[cfg(test)]
const DEFAULT_MAX_PARAMETER_VALUE_BYTES: usize = 4096;

#[derive(Clone, Copy)]
struct ValidatorConfiguration {
    validate_formats: bool,
    ignore_unknown_formats: bool,
    regex_size_limit: usize,
    regex_dfa_size_limit: usize,
}

const QUALIFIED_CONFIGURATION: ValidatorConfiguration = ValidatorConfiguration {
    validate_formats: false,
    ignore_unknown_formats: true,
    regex_size_limit: REGEX_SIZE_LIMIT,
    regex_dfa_size_limit: REGEX_DFA_SIZE_LIMIT,
};

#[derive(Clone, Copy)]
enum CardinalityComparison {
    Minimum,
    Maximum,
}

struct CardinalityKeyword {
    bound: ExactNonnegativeInteger,
    comparison: CardinalityComparison,
}

struct AnnotationKeyword;

impl Keyword for AnnotationKeyword {
    fn validate<'i>(&self, _instance: &'i Value) -> Result<(), ValidationError<'i>> {
        Ok(())
    }

    fn is_valid(&self, _instance: &Value) -> bool {
        true
    }
}

impl Keyword for CardinalityKeyword {
    fn validate<'i>(&self, instance: &'i Value) -> Result<(), ValidationError<'i>> {
        if self.is_valid(instance) {
            Ok(())
        } else {
            Err(ValidationError::custom(
                "array length violates its cardinality bound",
            ))
        }
    }

    fn is_valid(&self, instance: &Value) -> bool {
        let Some(values) = instance.as_array() else {
            return true;
        };
        match self.comparison {
            CardinalityComparison::Minimum => {
                self.bound.compare_usize(values.len()) != Ordering::Greater
            }
            CardinalityComparison::Maximum => {
                self.bound.compare_usize(values.len()) != Ordering::Less
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ExactNonnegativeInteger {
    significant_digits: String,
    scale: BigInt,
}

impl ExactNonnegativeInteger {
    fn parse(value: &Value) -> Option<Self> {
        let input = value.as_number()?.as_str();
        let (negative, unsigned) = match input.strip_prefix('-') {
            Some(unsigned) => (true, unsigned),
            None => (false, input),
        };
        let (significand, exponent) = match unsigned.find(['e', 'E']) {
            Some(index) => (&unsigned[..index], &unsigned[index + 1..]),
            None => (unsigned, "0"),
        };
        let exponent = exponent.parse::<BigInt>().ok()?;
        let (whole, fraction) = significand.split_once('.').unwrap_or((significand, ""));
        let mut digits = String::with_capacity(whole.len() + fraction.len());
        digits.push_str(whole);
        digits.push_str(fraction);
        if digits.bytes().all(|digit| digit == b'0') {
            return Some(Self {
                significant_digits: "0".to_owned(),
                scale: BigInt::from(0_u8),
            });
        }
        if negative {
            return None;
        }

        let digits = digits.trim_start_matches('0');
        let trailing_zeros = digits
            .bytes()
            .rev()
            .take_while(|digit| *digit == b'0')
            .count();
        let significant_digits = digits[..digits.len() - trailing_zeros].to_owned();
        let scale = exponent - BigInt::from(fraction.len()) + BigInt::from(trailing_zeros);
        (scale >= BigInt::from(0_u8)).then_some(Self {
            significant_digits,
            scale,
        })
    }

    fn compare_usize(&self, value: usize) -> Ordering {
        if self.significant_digits == "0" {
            return 0usize.cmp(&value);
        }
        let value = value.to_string();
        let digit_count = BigInt::from(self.significant_digits.len()) + &self.scale;
        match digit_count.cmp(&BigInt::from(value.len())) {
            Ordering::Equal => {
                let scale = self
                    .scale
                    .to_string()
                    .parse::<usize>()
                    .expect("a usize-sized decimal has a usize-sized scale");
                let mut bound = self.significant_digits.clone();
                bound.extend(std::iter::repeat_n('0', scale));
                bound.as_str().cmp(&value)
            }
            ordering => ordering,
        }
    }
}

fn min_items_factory<'a>(
    _parent: &'a serde_json::Map<String, Value>,
    value: &'a Value,
    _path: Location,
) -> Result<Box<dyn Keyword>, ValidationError<'a>> {
    cardinality_factory(value, CardinalityComparison::Minimum)
}

fn max_items_factory<'a>(
    _parent: &'a serde_json::Map<String, Value>,
    value: &'a Value,
    _path: Location,
) -> Result<Box<dyn Keyword>, ValidationError<'a>> {
    cardinality_factory(value, CardinalityComparison::Maximum)
}

fn annotation_factory<'a>(
    _parent: &'a serde_json::Map<String, Value>,
    _value: &'a Value,
    _path: Location,
) -> Result<Box<dyn Keyword>, ValidationError<'a>> {
    Ok(Box::new(AnnotationKeyword))
}

fn cardinality_factory(
    value: &Value,
    comparison: CardinalityComparison,
) -> Result<Box<dyn Keyword>, ValidationError<'static>> {
    let bound = ExactNonnegativeInteger::parse(value).ok_or_else(|| {
        ValidationError::schema("array cardinality bound must be a nonnegative integer")
    })?;
    Ok(Box::new(CardinalityKeyword { bound, comparison }))
}

pub(crate) struct Validator {
    inner: jsonschema::Validator,
    root_uri: RetrievalUri,
    anchor_locations: HashMap<String, String>,
    schema_location_aliases: HashMap<String, String>,
    cardinality_bound_limits: HashMap<SchemaLocation, Value>,
    #[cfg(schemaform_test_validation_faults)]
    injected_failure_after: Option<usize>,
}

pub(crate) enum Outcome {
    Valid,
    Invalid {
        findings: Vec<ValidationFinding>,
        truncated: bool,
    },
    Indeterminate(IndeterminateReason),
}

impl Validator {
    pub(crate) fn compile(graph: &ResourceGraph) -> Result<Self, QualificationError> {
        let root_uri = RetrievalUri::parse(graph.root_resource())
            .expect("qualified resource identities are absolute and fragment-free");
        let root_document = graph.validator_root_document();
        let inner = build_validator(&root_document, graph.root_resource(), graph.registry())
            .map_err(|error| {
                let (resource, pointer) = error.absolute_keyword_location().map_or_else(
                    || (None, error.schema_path().to_string()),
                    |absolute| {
                        let (resource, encoded_pointer) = absolute
                            .as_str()
                            .split_once('#')
                            .map_or((absolute.as_str(), ""), |parts| parts);
                        (
                            Some(resource),
                            crate::resources::decode_uri_fragment(encoded_pointer)
                                .unwrap_or_else(|| error.schema_path().to_string()),
                        )
                    },
                );
                QualificationError::InvalidSchema {
                    location: graph.qualification_location_for_error(
                        resource,
                        &pointer,
                        error.instance().as_ref(),
                    ),
                }
            })?;

        Ok(Self {
            inner,
            root_uri,
            anchor_locations: graph.anchor_locations().collect(),
            schema_location_aliases: graph.schema_location_aliases(),
            cardinality_bound_limits: graph.cardinality_bound_limits(),
            #[cfg(schemaform_test_validation_faults)]
            injected_failure_after: graph
                .root_document()
                .get("x-schemaform-test-validation-fault")
                .and_then(|value| match value {
                    Value::Bool(true) => Some(0),
                    Value::Number(value) => value.as_u64().and_then(|value| value.try_into().ok()),
                    _ => None,
                }),
        })
    }

    #[cfg(test)]
    pub(crate) fn validate(&self, form_data: &Value) -> Outcome {
        self.validate_with_limits(form_data, 256, DEFAULT_MAX_PARAMETER_VALUE_BYTES)
    }

    pub(crate) fn validate_with_limits(
        &self,
        form_data: &Value,
        max_retained_findings: usize,
        max_parameter_value_bytes: usize,
    ) -> Outcome {
        #[cfg(schemaform_test_validation_faults)]
        if self.injected_failure_after == Some(0) {
            return Outcome::Indeterminate(IndeterminateReason::new("injected-validator-failure"));
        }

        let mut findings = Vec::new();
        let mut truncated = false;
        let mut invalid = false;
        #[cfg(schemaform_test_validation_faults)]
        let mut translated_findings = 0usize;
        for error in self.inner.iter_errors(form_data) {
            if evaluator_failed(error.kind()) {
                return Outcome::Indeterminate(IndeterminateReason::new(
                    "validator-evaluation-failed",
                ));
            }
            invalid = true;
            let finding = self.translate(error, max_parameter_value_bytes);
            #[cfg(schemaform_test_validation_faults)]
            {
                translated_findings += 1;
                if self.injected_failure_after == Some(translated_findings) {
                    return Outcome::Indeterminate(IndeterminateReason::new(
                        "injected-validator-failure",
                    ));
                }
            }
            if findings.contains(&finding) {
                continue;
            }
            if findings.len() < max_retained_findings {
                findings.push(finding);
            } else {
                truncated = true;
                if findings.is_empty() {
                    continue;
                }
                let largest = findings
                    .iter()
                    .enumerate()
                    .max_by(|(_, left), (_, right)| compare_findings(left, right))
                    .map(|(index, _)| index)
                    .expect("the retained finding limit is nonzero");
                if compare_findings(&finding, &findings[largest]).is_lt() {
                    findings[largest] = finding;
                }
            }
        }

        if !invalid {
            return Outcome::Valid;
        }
        findings.sort_by(compare_findings);
        Outcome::Invalid {
            findings,
            truncated,
        }
    }

    fn translate(
        &self,
        error: ValidationError<'_>,
        max_parameter_value_bytes: usize,
    ) -> ValidationFinding {
        let mut keyword_location = self.schema_location(&error);
        let mut code = finding_code(error.kind());
        let mut parameters = match error.kind() {
            ValidationErrorKind::AdditionalItems { limit } => json!({ "limit": limit }),
            ValidationErrorKind::AdditionalProperties { unexpected }
            | ValidationErrorKind::UnevaluatedItems { unexpected }
            | ValidationErrorKind::UnevaluatedProperties { unexpected } => {
                json!({ "unexpectedCount": unexpected.len() })
            }
            ValidationErrorKind::Constant { expected_value } => {
                value_parameter("expected", expected_value, max_parameter_value_bytes)
            }
            ValidationErrorKind::ContentEncoding { content_encoding } => string_parameter(
                "contentEncoding",
                content_encoding,
                max_parameter_value_bytes,
            ),
            ValidationErrorKind::ContentMediaType { content_media_type } => string_parameter(
                "contentMediaType",
                content_media_type,
                max_parameter_value_bytes,
            ),
            ValidationErrorKind::Enum { options } => {
                json!({ "optionCount": options.as_array().map_or(0, Vec::len) })
            }
            ValidationErrorKind::ExclusiveMaximum { limit }
            | ValidationErrorKind::ExclusiveMinimum { limit }
            | ValidationErrorKind::Maximum { limit }
            | ValidationErrorKind::Minimum { limit } => {
                value_parameter("limit", limit, max_parameter_value_bytes)
            }
            ValidationErrorKind::Format { format } => {
                string_parameter("format", format, max_parameter_value_bytes)
            }
            ValidationErrorKind::MaxItems { limit } | ValidationErrorKind::MinItems { limit } => {
                self.cardinality_bound_limits
                    .get(&keyword_location)
                    .map_or_else(
                        || json!({ "limit": limit }),
                        |limit| value_parameter("limit", limit, max_parameter_value_bytes),
                    )
            }
            kind @ ValidationErrorKind::Custom { .. }
                if custom_cardinality_keyword(kind).is_some() =>
            {
                self.cardinality_bound_limits
                    .get(&keyword_location)
                    .map_or_else(
                        || json!({}),
                        |limit| value_parameter("limit", limit, max_parameter_value_bytes),
                    )
            }
            ValidationErrorKind::MaxLength { limit }
            | ValidationErrorKind::MaxProperties { limit }
            | ValidationErrorKind::MinLength { limit }
            | ValidationErrorKind::MinProperties { limit } => json!({ "limit": limit }),
            ValidationErrorKind::MultipleOf { multiple_of } => {
                value_parameter("multipleOf", multiple_of, max_parameter_value_bytes)
            }
            ValidationErrorKind::Pattern { pattern } => {
                string_parameter("pattern", pattern, max_parameter_value_bytes)
            }
            ValidationErrorKind::Required { property } => {
                value_parameter("property", property, max_parameter_value_bytes)
            }
            ValidationErrorKind::AnyOf { .. }
            | ValidationErrorKind::Contains
            | ValidationErrorKind::Custom { .. }
            | ValidationErrorKind::FalseSchema
            | ValidationErrorKind::FromUtf8 { .. }
            | ValidationErrorKind::Not { .. }
            | ValidationErrorKind::OneOfMultipleValid { .. }
            | ValidationErrorKind::OneOfNotValid { .. }
            | ValidationErrorKind::PropertyNames { .. }
            | ValidationErrorKind::Type { .. }
            | ValidationErrorKind::UniqueItems
            | ValidationErrorKind::Referencing(_)
            | ValidationErrorKind::BacktrackLimitExceeded { .. }
            | ValidationErrorKind::RegexEngineFailure { .. } => json!({}),
        };
        if matches!(error.kind(), ValidationErrorKind::Contains) {
            let bound = match error.schema_path().as_str() {
                pointer if pointer.ends_with("/minContains") => Some("minContains"),
                pointer if pointer.ends_with("/maxContains") => Some("maxContains"),
                _ => None,
            };
            if let Some(bound) = bound
                && let Some(bound_location) = contains_bound_location(&keyword_location, bound)
            {
                code = bound;
                keyword_location = bound_location;
                if let Some(limit) = self.cardinality_bound_limits.get(&keyword_location) {
                    parameters = value_parameter("limit", limit, max_parameter_value_bytes);
                }
            }
        }
        ValidationFinding::new(
            code,
            JsonPointer::parse(error.instance_path().to_string())
                .expect("validator instance locations are JSON Pointers"),
            keyword_location,
            parameters,
        )
    }

    fn schema_location(&self, error: &ValidationError<'_>) -> SchemaLocation {
        if let Some(absolute) = error.absolute_keyword_location() {
            if let Some((_, anchor_pointer, suffix)) = self
                .anchor_locations
                .iter()
                .filter_map(|(prefix, pointer)| {
                    absolute.as_str().strip_prefix(prefix).and_then(|suffix| {
                        (suffix.is_empty() || suffix.starts_with('/')).then_some((
                            prefix.len(),
                            pointer,
                            suffix,
                        ))
                    })
                })
                .max_by_key(|(prefix_length, _, _)| *prefix_length)
            {
                let resource = absolute
                    .as_str()
                    .split_once('#')
                    .map_or(absolute.as_str(), |(resource, _)| resource);
                let suffix = crate::resources::decode_uri_fragment(suffix)
                    .unwrap_or_else(|| suffix.to_owned());
                if let (Ok(resource), Ok(pointer)) = (
                    RetrievalUri::parse(resource),
                    JsonPointer::parse(format!("{anchor_pointer}{suffix}")),
                ) {
                    return SchemaLocation::new(resource, pointer);
                }
            }
            let (resource, encoded_pointer) = absolute
                .as_str()
                .split_once('#')
                .map_or((absolute.as_str(), None), |(resource, pointer)| {
                    (resource, Some(pointer))
                });
            let pointer = encoded_pointer
                .and_then(crate::resources::decode_uri_fragment)
                .unwrap_or_else(|| error.schema_path().to_string());
            let pointer = self
                .schema_location_aliases
                .get(&format!("{resource}#{pointer}"))
                .cloned()
                .unwrap_or(pointer);
            if let (Ok(resource), Ok(pointer)) =
                (RetrievalUri::parse(resource), JsonPointer::parse(pointer))
            {
                return SchemaLocation::new(resource, pointer);
            }
        }

        SchemaLocation::new(
            self.root_uri.clone(),
            JsonPointer::parse(error.schema_path().to_string())
                .expect("validator keyword locations are JSON Pointers"),
        )
    }
}

fn evaluator_failed(kind: &ValidationErrorKind) -> bool {
    match kind {
        ValidationErrorKind::BacktrackLimitExceeded { .. }
        | ValidationErrorKind::RegexEngineFailure { .. }
        | ValidationErrorKind::FromUtf8 { .. }
        | ValidationErrorKind::Referencing(_) => true,
        ValidationErrorKind::Custom { .. } => custom_cardinality_keyword(kind).is_none(),
        ValidationErrorKind::AnyOf { context }
        | ValidationErrorKind::OneOfMultipleValid { context }
        | ValidationErrorKind::OneOfNotValid { context } => context
            .iter()
            .flatten()
            .any(|error| evaluator_failed(error.kind())),
        ValidationErrorKind::PropertyNames { error } => evaluator_failed(error.kind()),
        _ => false,
    }
}

fn finding_code(kind: &ValidationErrorKind) -> &'static str {
    match kind {
        ValidationErrorKind::AdditionalItems { .. } => "additionalItems",
        ValidationErrorKind::AdditionalProperties { .. } => "additionalProperties",
        ValidationErrorKind::AnyOf { .. } => "anyOf",
        ValidationErrorKind::Constant { .. } => "const",
        ValidationErrorKind::Contains => "contains",
        ValidationErrorKind::ContentEncoding { .. } => "contentEncoding",
        ValidationErrorKind::ContentMediaType { .. } => "contentMediaType",
        ValidationErrorKind::Enum { .. } => "enum",
        ValidationErrorKind::ExclusiveMaximum { .. } => "exclusiveMaximum",
        ValidationErrorKind::ExclusiveMinimum { .. } => "exclusiveMinimum",
        ValidationErrorKind::FalseSchema => "falseSchema",
        ValidationErrorKind::Format { .. } => "format",
        ValidationErrorKind::MaxItems { .. } => "maxItems",
        ValidationErrorKind::Maximum { .. } => "maximum",
        ValidationErrorKind::MaxLength { .. } => "maxLength",
        ValidationErrorKind::MaxProperties { .. } => "maxProperties",
        ValidationErrorKind::MinItems { .. } => "minItems",
        ValidationErrorKind::Minimum { .. } => "minimum",
        ValidationErrorKind::MinLength { .. } => "minLength",
        ValidationErrorKind::MinProperties { .. } => "minProperties",
        ValidationErrorKind::MultipleOf { .. } => "multipleOf",
        ValidationErrorKind::Not { .. } => "not",
        ValidationErrorKind::OneOfMultipleValid { .. }
        | ValidationErrorKind::OneOfNotValid { .. } => "oneOf",
        ValidationErrorKind::Pattern { .. } => "pattern",
        ValidationErrorKind::PropertyNames { .. } => "propertyNames",
        ValidationErrorKind::Required { .. } => "required",
        ValidationErrorKind::Type { .. } => "type",
        ValidationErrorKind::UnevaluatedItems { .. } => "unevaluatedItems",
        ValidationErrorKind::UnevaluatedProperties { .. } => "unevaluatedProperties",
        ValidationErrorKind::UniqueItems => "uniqueItems",
        kind @ ValidationErrorKind::Custom { .. } => custom_cardinality_keyword(kind)
            .expect("unrelated custom failures are handled before finding translation"),
        ValidationErrorKind::BacktrackLimitExceeded { .. }
        | ValidationErrorKind::RegexEngineFailure { .. }
        | ValidationErrorKind::FromUtf8 { .. }
        | ValidationErrorKind::Referencing(_) => {
            unreachable!("evaluator failures are handled before finding translation")
        }
    }
}

fn custom_cardinality_keyword(kind: &ValidationErrorKind) -> Option<&'static str> {
    let ValidationErrorKind::Custom { keyword, .. } = kind else {
        return None;
    };
    match keyword.as_str() {
        "minItems" => Some("minItems"),
        "maxItems" => Some("maxItems"),
        _ => None,
    }
}

fn build_validator(
    data_schema: &Value,
    base_uri: &str,
    registry: &referencing::Registry<'_>,
) -> Result<jsonschema::Validator, ValidationError<'static>> {
    jsonschema::draft202012::options()
        .with_base_uri(base_uri)
        .with_registry(registry)
        .with_retriever(DenyRetrieval)
        .with_keyword("dependencies", annotation_factory)
        .with_keyword("additionalItems", annotation_factory)
        .with_keyword("$recursiveRef", annotation_factory)
        .with_keyword("minItems", min_items_factory)
        .with_keyword("maxItems", max_items_factory)
        .should_validate_formats(QUALIFIED_CONFIGURATION.validate_formats)
        .should_ignore_unknown_formats(QUALIFIED_CONFIGURATION.ignore_unknown_formats)
        .with_pattern_options(
            PatternOptions::regex()
                .size_limit(QUALIFIED_CONFIGURATION.regex_size_limit)
                .dfa_size_limit(QUALIFIED_CONFIGURATION.regex_dfa_size_limit),
        )
        .build(data_schema)
}

fn contains_bound_location(location: &SchemaLocation, keyword: &str) -> Option<SchemaLocation> {
    let parent = location.pointer().as_str().strip_suffix("/contains")?;
    let pointer = JsonPointer::parse(format!("{parent}/{keyword}")).ok()?;
    Some(SchemaLocation::new(location.resource().clone(), pointer))
}

fn compare_findings(left: &ValidationFinding, right: &ValidationFinding) -> Ordering {
    left.instance_location()
        .cmp(right.instance_location())
        .then_with(|| {
            left.keyword_location()
                .resource()
                .cmp(right.keyword_location().resource())
        })
        .then_with(|| {
            left.keyword_location()
                .pointer()
                .cmp(right.keyword_location().pointer())
        })
        .then_with(|| left.code().cmp(right.code()))
        .then_with(|| {
            left.parameters()
                .to_string()
                .cmp(&right.parameters().to_string())
        })
}

fn value_parameter(name: &str, value: &Value, maximum: usize) -> Value {
    if serde_json::to_vec(value).is_ok_and(|encoded| encoded.len() <= maximum) {
        Value::Object(serde_json::Map::from_iter([(
            name.to_owned(),
            value.clone(),
        )]))
    } else {
        json!({ "omitted": true })
    }
}

fn string_parameter(name: &str, value: &str, maximum: usize) -> Value {
    value_parameter(name, &Value::String(value.to_owned()), maximum)
}

pub(crate) fn default_root_uri() -> RetrievalUri {
    RetrievalUri::parse(DEFAULT_ROOT_RETRIEVAL_URI)
        .expect("the built-in root retrieval URI is absolute and fragment-free")
}

#[cfg(test)]
mod tests {
    use referencing::{Draft, Registry};
    use serde_json::json;

    use super::*;

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn validator_construction_uses_the_qualified_configuration() {
        const {
            assert!(!QUALIFIED_CONFIGURATION.validate_formats);
            assert!(QUALIFIED_CONFIGURATION.ignore_unknown_formats);
        }
        assert_eq!(QUALIFIED_CONFIGURATION.regex_size_limit, 10 * 1024 * 1024);
        assert_eq!(
            QUALIFIED_CONFIGURATION.regex_dfa_size_limit,
            2 * 1024 * 1024
        );
    }

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn validator_construction_denies_resource_retrieval() {
        let registry = Registry::new()
            .draft(Draft::Draft202012)
            .prepare()
            .expect("the empty registry should prepare");
        let result = build_validator(
            &json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "$ref": "https://schemas.example/missing.json"
            }),
            "https://schemas.example/root.json",
            &registry,
        );
        let Err(error) = result else {
            panic!("validator construction should deny missing resource retrieval");
        };

        let ValidationErrorKind::Referencing(referencing::Error::Unretrievable { uri, source }) =
            error.kind()
        else {
            panic!("validator construction returned a non-retrieval error: {error}");
        };
        assert_eq!(uri, "https://schemas.example/missing.json");
        assert_eq!(
            source
                .downcast_ref::<crate::resources::RetrievalDenied>()
                .map(|denied| denied.0.as_str()),
            Some("https://schemas.example/missing.json")
        );
    }

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn exact_cardinality_bound_parser_accepts_every_integral_json_number_form() {
        for (input, expected_digits, expected_scale) in [
            ("0", "0", "0"),
            ("-0e999999999999999999999", "0", "0"),
            ("2.0", "2", "0"),
            ("120e-1", "12", "0"),
            ("1e4096", "1", "4096"),
            ("10e4095", "1", "4096"),
        ] {
            let value = serde_json::from_str(input).expect("the JSON number should parse");
            let parsed = ExactNonnegativeInteger::parse(&value)
                .unwrap_or_else(|| panic!("{input} should be a nonnegative integer"));
            assert_eq!(parsed.significant_digits, expected_digits, "{input}");
            assert_eq!(parsed.scale.to_string(), expected_scale, "{input}");
        }

        for input in ["-1", "1.5", "12e-1", "true"] {
            let value = serde_json::from_str(input).expect("the JSON value should parse");
            assert!(
                ExactNonnegativeInteger::parse(&value).is_none(),
                "{input} should not be a nonnegative integer"
            );
        }
    }

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn exact_cardinality_keywords_apply_only_to_arrays_across_machine_boundaries() {
        let registry = Registry::new()
            .draft(Draft::Draft202012)
            .prepare()
            .expect("the empty registry should prepare");
        for (keyword, limit, instance, valid) in [
            ("minItems", "2.0", json!([]), false),
            ("minItems", "2.0", json!([null, null]), true),
            ("maxItems", "2e0", json!([null, null, null]), false),
            ("maxItems", "2e0", json!([null, null]), true),
            ("minItems", "18446744073709551615", json!([]), false),
            ("maxItems", "18446744073709551615", json!([null]), true),
            ("minItems", "18446744073709551616", json!([]), false),
            ("maxItems", "18446744073709551616", json!([null]), true),
            ("minItems", "1e4096", json!({ "not": "an array" }), true),
            ("maxItems", "1e4096", json!("not an array"), true),
        ] {
            let schema = serde_json::from_str(&format!(
                r#"{{"$schema":"https://json-schema.org/draft/2020-12/schema","{keyword}":{limit}}}"#
            ))
            .expect("the cardinality schema should parse");
            let validator = build_validator(
                &schema,
                "https://schemas.example/cardinality.json",
                &registry,
            )
            .expect("the cardinality schema should compile");
            assert_eq!(
                validator.is_valid(&instance),
                valid,
                "unexpected {keyword} {limit} verdict for {instance}"
            );
            if !valid {
                let error = validator
                    .validate(&instance)
                    .expect_err("the invalid instance should produce an error");
                assert_eq!(custom_cardinality_keyword(error.kind()), Some(keyword));
            }
        }
    }

    fn dual_role_reference_schema(role: &str, keyword: &str) -> Value {
        let keyword_value = match keyword {
            "dependencies" => json!({ "foo": ["bar"] }),
            "additionalItems" => Value::Bool(false),
            "$recursiveRef" => Value::String("#".to_owned()),
            _ => serde_json::from_str::<Value>("1e4096")
                .expect("the arbitrary-precision bound should parse"),
        };
        let (assertion, reference) = match role {
            "const" => (
                json!({ "const": { (keyword): keyword_value } }),
                "#/x-opaque/const".to_owned(),
            ),
            "enum" => (
                json!({ "enum": [{ (keyword): keyword_value }] }),
                "#/x-opaque/enum/0".to_owned(),
            ),
            _ => unreachable!("the test only uses const and enum"),
        };
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "configuration": {
                    "allOf": [
                        { "$ref": "#/x-opaque" },
                        { "$ref": reference }
                    ]
                }
            },
            "x-opaque": assertion
        })
    }

    fn assert_dual_role_reference_is_not_rewritten(role: &str, keyword: &str) {
        let graph = ResourceGraph::prepare(
            RetrievalUri::parse("https://schemas.example/dual-role.json").unwrap(),
            dual_role_reference_schema(role, keyword),
            Vec::new(),
        )
        .expect("the dual-role schema should prepare");
        let validator = Validator::compile(&graph).expect("the dual-role schema should compile");
        let keyword_value = match keyword {
            "dependencies" => json!({ "foo": ["bar"] }),
            "additionalItems" => Value::Bool(false),
            "$recursiveRef" => Value::String("#".to_owned()),
            _ => serde_json::from_str::<Value>("1e4096")
                .expect("the arbitrary-precision instance number should parse"),
        };

        assert!(matches!(
            validator.validate(&json!({ "configuration": { (keyword): keyword_value } })),
            Outcome::Valid
        ));
    }

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn const_value_referenced_as_a_schema_is_not_rewritten() {
        assert_dual_role_reference_is_not_rewritten("const", "minItems");
    }

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn enum_value_referenced_as_a_schema_is_not_rewritten() {
        assert_dual_role_reference_is_not_rewritten("enum", "maxItems");
    }

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn legacy_keyword_values_referenced_as_schemas_are_not_rewritten() {
        for role in ["const", "enum"] {
            for keyword in ["dependencies", "additionalItems", "$recursiveRef"] {
                assert_dual_role_reference_is_not_rewritten(role, keyword);
            }
        }
    }

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn string_parameters_are_bounded_by_their_json_encoding() {
        assert_eq!(
            string_parameter(
                "pattern",
                &"a".repeat(DEFAULT_MAX_PARAMETER_VALUE_BYTES - 2),
                DEFAULT_MAX_PARAMETER_VALUE_BYTES,
            ),
            json!({ "pattern": "a".repeat(DEFAULT_MAX_PARAMETER_VALUE_BYTES - 2) })
        );
        assert_eq!(
            string_parameter(
                "pattern",
                &"\"".repeat(DEFAULT_MAX_PARAMETER_VALUE_BYTES / 2),
                DEFAULT_MAX_PARAMETER_VALUE_BYTES,
            ),
            json!({ "omitted": true })
        );
    }

    #[cfg_attr(
        all(target_arch = "wasm32", target_os = "unknown"),
        wasm_bindgen_test::wasm_bindgen_test
    )]
    #[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
    fn dependency_failures_with_messages_are_not_translated_to_findings() {
        let from_utf8 = String::from_utf8(vec![0xff]).expect_err("the fixture should be invalid");
        for kind in [
            ValidationErrorKind::FromUtf8 { error: from_utf8 },
            ValidationErrorKind::RegexEngineFailure {
                message: "dependency-message-sentinel".to_owned(),
            },
            ValidationErrorKind::Custom {
                keyword: "dependency-keyword-sentinel".to_owned(),
                message: "dependency-message-sentinel".to_owned(),
            },
        ] {
            assert!(evaluator_failed(&kind));
        }
        for keyword in ["minItems", "maxItems"] {
            let kind = ValidationErrorKind::Custom {
                keyword: keyword.to_owned(),
                message: "product-cardinality-sentinel".to_owned(),
            };
            assert!(!evaluator_failed(&kind));
            assert_eq!(finding_code(&kind), keyword);
        }
    }
}
