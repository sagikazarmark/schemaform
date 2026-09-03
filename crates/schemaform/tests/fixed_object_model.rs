use std::collections::{BTreeMap, BTreeSet};

use num_bigint::BigInt;
use schemaform::{
    DataRevision, DefinitionFingerprint, ExternalFinding, ExternalFindingBatch, FindingView, Form,
    FormDefinition, InstanceIdentity, JsonPointer, SubmissionOutcome, SubmissionSnapshot,
    Transition,
    form::{
        FindingVisibility, FindingVisibilityPolicy, ParseBlockerKind, SubmissionBlocker,
        TransactionError, ValidationOutcomeView,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const REPLAYS: [(&str, &str); 4] = [
    (
        "exact-number",
        include_str!("replays/fixed_object/exact-number.json"),
    ),
    (
        "rollback",
        include_str!("replays/fixed_object/rollback.json"),
    ),
    (
        "finding-order",
        include_str!("replays/fixed_object/finding-order.json"),
    ),
    (
        "lifecycle",
        include_str!("replays/fixed_object/lifecycle.json"),
    ),
];
const BINDINGS: [&str; 6] = [
    "/amount",
    "/details",
    "/details/label",
    "/name",
    "/nullable",
    "/quantity",
];

#[derive(Clone, Debug, Deserialize, Serialize)]
enum Command {
    InputName(String),
    InputAmount(String),
    InputQuantity(String),
    SetNullable(NullableCommand),
    MaterializeDetails,
    RemoveDetails,
    InputDetail(String),
    Blur(Target),
    Host(HostCommand),
    External {
        fresh: bool,
        source: Source,
        target: Target,
        blocking: bool,
    },
    Visibility {
        validation: Visibility,
        external: Visibility,
    },
    Reset,
    Reinitialize(ReinitializeCommand),
    Submit,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum NullableCommand {
    Null,
    Remove,
    Text,
    Repair,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum HostCommand {
    Commit,
    CommitInvalidData,
    Repair,
    BreakNullable,
    EquivalentQuantity,
    ClosureFailed,
    InvalidOperation,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum ReinitializeCommand {
    Valid,
    Invalid,
    Equal,
    Rejected,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum Source {
    Policy,
    Server,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
enum Target {
    Root,
    Amount,
    Details,
    Detail,
    Name,
    Quantity,
}

impl Target {
    fn binding(self) -> &'static str {
        match self {
            Self::Root => "",
            Self::Amount => "/amount",
            Self::Details => "/details",
            Self::Detail => "/details/label",
            Self::Name => "/name",
            Self::Quantity => "/quantity",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum Visibility {
    Immediate,
    TouchedOrSubmission,
    SubmissionOnly,
}

impl From<Visibility> for FindingVisibility {
    fn from(value: Visibility) -> Self {
        match value {
            Visibility::Immediate => Self::Immediate,
            Visibility::TouchedOrSubmission => Self::TouchedOrSubmission,
            Visibility::SubmissionOnly => Self::SubmissionOnly,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExternalModel {
    target: &'static str,
    blocking: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Model {
    data: Value,
    baseline: Value,
    buffers: BTreeMap<&'static str, String>,
    blockers: BTreeMap<&'static str, ParseBlockerKind>,
    touched: BTreeSet<&'static str>,
    external: BTreeMap<&'static str, ExternalModel>,
    validation_visibility: Visibility,
    external_visibility: Visibility,
    submission_attempted: bool,
}

#[derive(Clone, Debug)]
struct ModelStep {
    accepted: bool,
    force_data_revision: bool,
    force_state_revision: bool,
    submission: Option<Vec<FindingKey>>,
}

impl ModelStep {
    fn accepted() -> Self {
        Self {
            accepted: true,
            force_data_revision: false,
            force_state_revision: false,
            submission: None,
        }
    }

    fn rejected() -> Self {
        Self {
            accepted: false,
            ..Self::accepted()
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct NodeState {
    buffer: Option<String>,
    blocker: Option<ParseBlockerKind>,
    touched: bool,
    dirty: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum FindingKey {
    Parse(String, ParseBlockerKind),
    Validation {
        instance: String,
        resource: String,
        keyword: String,
        code: String,
        parameters: Value,
    },
    External(String, String, String, bool, Value),
}

fn validation_key(instance: &str, keyword: &str, code: &str, parameters: Value) -> FindingKey {
    FindingKey::Validation {
        instance: instance.to_owned(),
        resource: "urn:schemaform:root".to_owned(),
        keyword: keyword.to_owned(),
        code: code.to_owned(),
        parameters,
    }
}

impl Model {
    fn new() -> Self {
        let data =
            exact_json(r#"{"amount":1.2500,"name":"Ada","quantity":1e3,"nullable":"present"}"#);
        Self {
            baseline: data.clone(),
            data,
            buffers: BTreeMap::new(),
            blockers: BTreeMap::new(),
            touched: BTreeSet::new(),
            external: BTreeMap::new(),
            validation_visibility: Visibility::TouchedOrSubmission,
            external_visibility: Visibility::TouchedOrSubmission,
            submission_attempted: false,
        }
    }

    fn apply(&mut self, command: &Command) -> ModelStep {
        match command {
            Command::InputName(text) => {
                self.input_string("/name", text);
                ModelStep::accepted()
            }
            Command::InputAmount(text) => {
                self.buffers.insert("/amount", text.clone());
                match serde_json::from_str(text) {
                    Ok(value @ Value::Number(_)) => {
                        self.blockers.remove("/amount");
                        if !optional_semantic_equal(self.data.pointer("/amount"), Some(&value)) {
                            set_member(&mut self.data, "amount", value);
                            self.external.clear();
                        }
                    }
                    _ => {
                        self.blockers
                            .insert("/amount", ParseBlockerKind::InvalidNumber);
                    }
                }
                ModelStep::accepted()
            }
            Command::InputQuantity(text) => {
                self.buffers.insert("/quantity", text.clone());
                match parse_integer(text) {
                    Ok(integer) => {
                        self.blockers.remove("/quantity");
                        let value = exact_json(&integer.to_string());
                        if !optional_semantic_equal(self.data.pointer("/quantity"), Some(&value)) {
                            set_member(&mut self.data, "quantity", value);
                            self.external.clear();
                        }
                    }
                    Err(blocker) => {
                        self.blockers.insert("/quantity", blocker);
                    }
                }
                ModelStep::accepted()
            }
            Command::SetNullable(action) => self.apply_nullable(*action),
            Command::MaterializeDetails => {
                if self.data.get("details").is_some() {
                    return ModelStep::rejected();
                }
                set_member(&mut self.data, "details", json!({}));
                self.external.clear();
                ModelStep::accepted()
            }
            Command::RemoveDetails => {
                if self.data.get("details").is_none() {
                    return ModelStep::rejected();
                }
                remove_member(&mut self.data, "details");
                self.clear_edit_state("/details");
                self.external.clear();
                ModelStep::accepted()
            }
            Command::InputDetail(text) => {
                if !self.data.get("details").is_some_and(Value::is_object) {
                    return ModelStep::rejected();
                }
                self.input_string("/details/label", text);
                ModelStep::accepted()
            }
            Command::Blur(target) => {
                let binding = target.binding();
                if binding.is_empty() || binding == "/details" {
                    return ModelStep::rejected();
                }
                if !self.blockers.contains_key(binding) {
                    self.buffers.remove(binding);
                }
                self.touched.insert(binding);
                ModelStep::accepted()
            }
            Command::Host(action) => self.apply_host(*action),
            Command::External {
                fresh,
                source,
                target,
                blocking,
            } => {
                if !fresh {
                    return ModelStep::rejected();
                }
                self.external.insert(
                    source.name(),
                    ExternalModel {
                        target: target.binding(),
                        blocking: *blocking,
                    },
                );
                ModelStep::accepted()
            }
            Command::Visibility {
                validation,
                external,
            } => {
                self.validation_visibility = *validation;
                self.external_visibility = *external;
                ModelStep::accepted()
            }
            Command::Reset => {
                if !semantic_equal(&self.data, &self.baseline) {
                    self.data = self.baseline.clone();
                    self.external.clear();
                }
                self.buffers.clear();
                self.blockers.clear();
                self.touched.clear();
                self.submission_attempted = false;
                ModelStep::accepted()
            }
            Command::Reinitialize(kind) => self.reinitialize(*kind),
            Command::Submit => {
                self.buffers
                    .retain(|binding, _| self.blockers.contains_key(binding));
                self.submission_attempted = true;
                let mut step = ModelStep::accepted();
                step.submission = Some(self.submission_blockers());
                step
            }
        }
    }

    fn apply_nullable(&mut self, action: NullableCommand) -> ModelStep {
        match action {
            NullableCommand::Null => {
                if self.data.get("nullable") == Some(&Value::Null) {
                    return ModelStep::rejected();
                }
                set_member(&mut self.data, "nullable", Value::Null);
            }
            NullableCommand::Remove => {
                if self.data.get("nullable").is_none() {
                    return ModelStep::rejected();
                }
                remove_member(&mut self.data, "nullable");
            }
            NullableCommand::Text => {
                if self
                    .data
                    .get("nullable")
                    .is_some_and(|value| !value.is_string() && !value.is_null())
                {
                    return ModelStep::rejected();
                }
                self.input_string("/nullable", "repaired");
                return ModelStep::accepted();
            }
            NullableCommand::Repair => {
                if !self
                    .data
                    .get("nullable")
                    .is_some_and(|value| !value.is_string() && !value.is_null())
                {
                    return ModelStep::rejected();
                }
                set_member(&mut self.data, "nullable", json!("repaired"));
            }
        }
        self.clear_edit_state("/nullable");
        self.external.clear();
        ModelStep::accepted()
    }

    fn apply_host(&mut self, action: HostCommand) -> ModelStep {
        if matches!(
            action,
            HostCommand::ClosureFailed | HostCommand::InvalidOperation
        ) {
            return ModelStep::rejected();
        }
        let before = self.data.clone();
        match action {
            HostCommand::Commit => {
                set_member(&mut self.data, "name", json!("Host"));
                set_member(&mut self.data, "quantity", json!(2));
                self.clear_edit_state("/name");
                self.clear_edit_state("/quantity");
            }
            HostCommand::CommitInvalidData => {
                if self.data.get("name").is_none() {
                    return ModelStep::rejected();
                }
                remove_member(&mut self.data, "name");
                self.clear_edit_state("/name");
            }
            HostCommand::Repair => {
                set_member(&mut self.data, "name", json!("Repaired"));
                set_member(&mut self.data, "quantity", json!(3));
                self.clear_edit_state("/name");
                self.clear_edit_state("/quantity");
            }
            HostCommand::BreakNullable => {
                set_member(&mut self.data, "nullable", json!(7));
                self.clear_edit_state("/nullable");
            }
            HostCommand::EquivalentQuantity => {
                set_member(&mut self.data, "quantity", json!(1000));
                self.clear_edit_state("/quantity");
            }
            HostCommand::ClosureFailed | HostCommand::InvalidOperation => unreachable!(),
        }
        preserve_semantically_equal(&before, &mut self.data);
        if !semantic_equal(&before, &self.data) {
            self.external.clear();
        }
        ModelStep::accepted()
    }

    fn reinitialize(&mut self, kind: ReinitializeCommand) -> ModelStep {
        if matches!(kind, ReinitializeCommand::Rejected) {
            return ModelStep::rejected();
        }
        let data = match kind {
            ReinitializeCommand::Valid => json!({
                "amount": 1.5,
                "name": "Lin",
                "quantity": 7,
                "nullable": null
            }),
            ReinitializeCommand::Invalid => json!({ "amount": 2.5, "name": "", "quantity": 0 }),
            ReinitializeCommand::Equal => self.data.clone(),
            ReinitializeCommand::Rejected => unreachable!(),
        };
        self.data = data.clone();
        self.baseline = data;
        self.buffers.clear();
        self.blockers.clear();
        self.touched.clear();
        self.external.clear();
        self.submission_attempted = false;
        ModelStep {
            force_data_revision: true,
            force_state_revision: true,
            ..ModelStep::accepted()
        }
    }

    fn input_string(&mut self, binding: &'static str, text: &str) {
        let before = self.data.clone();
        self.buffers.insert(binding, text.to_owned());
        let member = binding.rsplit('/').next().expect("a property binding");
        if binding == "/details/label" {
            self.data["details"][member] = json!(text);
        } else {
            set_member(&mut self.data, member, json!(text));
        }
        if !semantic_equal(&before, &self.data) {
            self.external.clear();
        }
    }

    fn clear_edit_state(&mut self, prefix: &str) {
        self.buffers
            .retain(|binding, _| !binding.starts_with(prefix));
        self.blockers
            .retain(|binding, _| !binding.starts_with(prefix));
    }

    fn validation_findings(&self) -> Vec<FindingKey> {
        let mut findings = Vec::new();
        if self.data.get("amount").is_none() {
            findings.push(validation_key(
                "",
                "/required",
                "required",
                json!({ "property": "amount" }),
            ));
        }
        match self.data.get("name") {
            None => findings.push(validation_key(
                "",
                "/required",
                "required",
                json!({ "property": "name" }),
            )),
            Some(Value::String(name)) if name.chars().count() < 2 => {
                findings.push(validation_key(
                    "/name",
                    "/properties/name/minLength",
                    "minLength",
                    json!({ "limit": 2 }),
                ));
            }
            _ => {}
        }
        if let Some(number) = self.data.get("quantity").and_then(Value::as_number)
            && parse_integer(&number.to_string()).is_ok_and(|value| value < 1.into())
        {
            findings.push(validation_key(
                "/quantity",
                "/properties/quantity/minimum",
                "minimum",
                json!({ "limit": 1 }),
            ));
        }
        if self
            .data
            .get("nullable")
            .is_some_and(|value| !value.is_string() && !value.is_null())
        {
            findings.push(validation_key(
                "/nullable",
                "/properties/nullable/type",
                "type",
                json!({}),
            ));
        }
        if let Some(label) = self.data.pointer("/details/label").and_then(Value::as_str)
            && label.chars().count() < 2
        {
            findings.push(validation_key(
                "/details/label",
                "/properties/details/properties/label/minLength",
                "minLength",
                json!({ "limit": 2 }),
            ));
        }
        findings.sort_by(|left, right| match (left, right) {
            (
                FindingKey::Validation {
                    instance: left_instance,
                    resource: left_resource,
                    keyword: left_keyword,
                    code: left_code,
                    parameters: left_parameters,
                },
                FindingKey::Validation {
                    instance: right_instance,
                    resource: right_resource,
                    keyword: right_keyword,
                    code: right_code,
                    parameters: right_parameters,
                },
            ) => left_instance
                .cmp(right_instance)
                .then_with(|| left_resource.cmp(right_resource))
                .then_with(|| left_keyword.cmp(right_keyword))
                .then_with(|| left_code.cmp(right_code))
                .then_with(|| {
                    left_parameters
                        .to_string()
                        .cmp(&right_parameters.to_string())
                }),
            _ => unreachable!("only validation findings are sorted here"),
        });
        findings
    }

    fn visible_findings(&self) -> Vec<FindingKey> {
        let mut findings = self
            .validation_findings()
            .into_iter()
            .filter(|finding| {
                let FindingKey::Validation { instance, .. } = finding else {
                    unreachable!()
                };
                self.visible(self.validation_visibility, instance)
            })
            .collect::<Vec<_>>();
        findings.extend(
            self.external
                .iter()
                .filter(|(_, finding)| self.visible(self.external_visibility, finding.target))
                .map(|(source, finding)| {
                    FindingKey::External(
                        (*source).to_owned(),
                        finding.target.to_owned(),
                        "host-finding".to_owned(),
                        finding.blocking,
                        json!({}),
                    )
                }),
        );
        findings.extend(
            self.blockers
                .iter()
                .map(|(binding, blocker)| FindingKey::Parse((*binding).to_owned(), *blocker)),
        );
        findings
    }

    fn visible(&self, policy: Visibility, binding: &str) -> bool {
        match policy {
            Visibility::Immediate => true,
            Visibility::TouchedOrSubmission => {
                self.submission_attempted || self.touched.contains(binding)
            }
            Visibility::SubmissionOnly => self.submission_attempted,
        }
    }

    fn submission_blockers(&self) -> Vec<FindingKey> {
        let mut blockers = self
            .blockers
            .iter()
            .map(|(binding, blocker)| FindingKey::Parse((*binding).to_owned(), *blocker))
            .collect::<Vec<_>>();
        blockers.extend(self.validation_findings());
        blockers.extend(
            self.external
                .iter()
                .filter(|(_, finding)| finding.blocking)
                .map(|(source, finding)| {
                    FindingKey::External(
                        (*source).to_owned(),
                        finding.target.to_owned(),
                        "host-finding".to_owned(),
                        true,
                        json!({}),
                    )
                }),
        );
        blockers
    }

    fn node_state(&self, binding: &'static str) -> NodeState {
        NodeState {
            buffer: self.buffers.get(binding).cloned(),
            blocker: self.blockers.get(binding).copied(),
            touched: self.touched.contains(binding),
            dirty: !optional_semantic_equal(
                self.data.pointer(binding),
                self.baseline.pointer(binding),
            ),
        }
    }
}

impl Source {
    fn name(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::Server => "server",
        }
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn checked_in_minimized_replays_match_the_reference_model() {
    for (name, source) in REPLAYS {
        let trace: Vec<Command> =
            serde_json::from_str(source).expect("the replay trace should parse");
        run_trace(&trace).unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
#[test]
#[ignore = "expensive property test run explicitly in CI"]
fn generated_fixed_object_traces_match_the_reference_model() {
    use proptest::{
        prelude::*,
        test_runner::{Config, FileFailurePersistence, RngSeed, TestRunner},
    };

    let cases = match std::env::var("SCHEMAFORM_PROPTEST_PROFILE").as_deref() {
        Ok("nightly") => 100_000,
        Ok("release") => 1_000_000,
        Ok("pr") | Err(_) => 10_000,
        Ok(other) => panic!("unknown SCHEMAFORM_PROPTEST_PROFILE value: {other}"),
    };
    let config = Config {
        cases,
        rng_seed: RngSeed::Fixed(0x5eed_f198_2026_0722),
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/replays/fixed_object/proptest-seeds.txt",
        ))),
        ..Config::default()
    };
    let mut runner = TestRunner::new(config);
    let definition = definition();
    runner
        .run(
            &proptest::collection::vec(command_strategy(), 1..=256),
            |trace| {
                run_trace_with_definition(&definition, &trace).map_err(|error| {
                    TestCaseError::fail(format!(
                        "{error}\nminimized trace can be promoted as JSON:\n{}",
                        serde_json::to_string_pretty(&trace)
                            .expect("trace serialization should work")
                    ))
                })
            },
        )
        .unwrap();
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn command_strategy() -> impl proptest::strategy::Strategy<Value = Command> {
    use proptest::prelude::*;

    let target = prop_oneof![
        Just(Target::Amount),
        Just(Target::Details),
        Just(Target::Detail),
        Just(Target::Name),
        Just(Target::Quantity),
        Just(Target::Root),
    ];
    let visibility = prop_oneof![
        Just(Visibility::Immediate),
        Just(Visibility::TouchedOrSubmission),
        Just(Visibility::SubmissionOnly),
    ];
    prop_oneof![
        4 => prop_oneof![Just(""), Just("A"), Just("Ada"), Just("Grace")]
            .prop_map(|value| Command::InputName(value.to_owned())),
        6 => prop_oneof![
            Just("-"), Just("1e"), Just("0"), Just("1.25"), Just("1.2500"),
            Just("125e-2"), Just("2.500000000000000000000000000000000001")
        ].prop_map(|value| Command::InputAmount(value.to_owned())),
        8 => prop_oneof![
            Just("-"), Just("1e"), Just("0"), Just("1"), Just("1000"), Just("1e3"),
            Just("1.2"), Just("12e-1"), Just("1.00"), Just("101e-2"),
            Just("1e+3"), Just("1e-3"), Just("10.00e2"), Just("-0"),
            Just("1234567890123456789012345678901234567890")
        ].prop_map(|value| Command::InputQuantity(value.to_owned())),
        1 => Just(Command::InputQuantity(format!("1{}", "0".repeat(4095)))),
        1 => Just(Command::InputQuantity(format!("1{}", "0".repeat(4096)))),
        3 => prop_oneof![
            Just(NullableCommand::Null), Just(NullableCommand::Remove),
            Just(NullableCommand::Text), Just(NullableCommand::Repair)
        ].prop_map(Command::SetNullable),
        2 => Just(Command::MaterializeDetails),
        2 => Just(Command::RemoveDetails),
        3 => prop_oneof![Just(""), Just("x"), Just("detail")]
            .prop_map(|value| Command::InputDetail(value.to_owned())),
        3 => target.clone().prop_map(Command::Blur),
        5 => prop_oneof![
            Just(HostCommand::Commit), Just(HostCommand::CommitInvalidData),
            Just(HostCommand::Repair), Just(HostCommand::BreakNullable),
            Just(HostCommand::EquivalentQuantity), Just(HostCommand::ClosureFailed),
            Just(HostCommand::InvalidOperation)
        ].prop_map(Command::Host),
        4 => (any::<bool>(), prop_oneof![Just(Source::Policy), Just(Source::Server)], target, any::<bool>())
            .prop_map(|(fresh, source, target, blocking)| Command::External { fresh, source, target, blocking }),
        2 => (visibility.clone(), visibility).prop_map(|(validation, external)| Command::Visibility { validation, external }),
        2 => Just(Command::Reset),
        3 => prop_oneof![
            Just(ReinitializeCommand::Valid), Just(ReinitializeCommand::Invalid),
            Just(ReinitializeCommand::Equal), Just(ReinitializeCommand::Rejected)
        ].prop_map(Command::Reinitialize),
        3 => Just(Command::Submit),
    ]
}

fn run_trace(trace: &[Command]) -> Result<(), String> {
    let definition = definition();
    run_trace_with_definition(&definition, trace)
}

fn run_trace_with_definition(definition: &FormDefinition, trace: &[Command]) -> Result<(), String> {
    let mut form = definition
        .create_form(Model::new().data.clone())
        .map_err(|error| error.to_string())?;
    let mut model = Model::new();
    let mut retained_snapshots = Vec::new();
    let root_identity = form.view().root();
    let expected_identities = fixed_identities(&form);
    assert_observation(&form, &model).map_err(|error| format!("initial state: {error}"))?;

    for (index, command) in trace.iter().enumerate() {
        let before_model = model.clone();
        let before_data_revision = form.view().data_revision();
        let before_state_revision = form.view().state_revision();
        let expected = model.apply(command);
        let actual = execute(&mut form, definition, command, &mut retained_snapshots);
        if expected.accepted != actual.accepted {
            return Err(format!(
                "command {index} {command:?}: acceptance mismatch, expected {}, got {}",
                expected.accepted, actual.accepted
            ));
        }
        if let Some(transition) = actual.transition.as_ref() {
            if transition.before_data_revision() != before_data_revision
                || transition.before_state_revision() != before_state_revision
                || transition.after_data_revision() != form.view().data_revision()
                || transition.after_state_revision() != form.view().state_revision()
            {
                return Err(format!(
                    "command {index} {command:?}: transition revisions do not bracket the published state"
                ));
            }
            let expected_data_delta =
                expected.force_data_revision || !semantic_equal(&before_model.data, &model.data);
            let expected_state_delta = expected.force_state_revision || before_model != model;
            if (transition.before_data_revision() != transition.after_data_revision())
                != expected_data_delta
                || (transition.before_state_revision() != transition.after_state_revision())
                    != expected_state_delta
            {
                return Err(format!(
                    "command {index} {command:?}: revision delta mismatch; expected data={expected_data_delta}, state={expected_state_delta}"
                ));
            }
            if transition.removed().next().is_some() {
                return Err(format!(
                    "command {index} {command:?}: fixed-object identity was removed"
                ));
            }
            let mut changed = Vec::new();
            for identity in transition.changed() {
                if changed.contains(&identity)
                    || (identity != root_identity
                        && !expected_identities
                            .iter()
                            .any(|(_, expected)| *expected == identity))
                {
                    return Err(format!(
                        "command {index} {command:?}: transition contains an unknown or duplicate changed identity"
                    ));
                }
                changed.push(identity);
            }
        } else if form.view().data_revision() != before_data_revision
            || form.view().state_revision() != before_state_revision
        {
            return Err(format!(
                "command {index} {command:?}: rejected operation mutated revisions"
            ));
        }
        assert_fixed_identities(&form, root_identity, &expected_identities)
            .map_err(|error| format!("command {index} {command:?}: {error}"))?;
        if let Some(expected_blockers) = expected.submission
            && actual.submission != Some(expected_blockers)
        {
            return Err(format!(
                "command {index} {command:?}: submission mismatch; expected {:?}, got {:?}",
                model.submission_blockers(),
                actual.submission
            ));
        }
        assert_observation(&form, &model)
            .map_err(|error| format!("command {index} {command:?}: {error}"))?;
        assert_retained_snapshots(&retained_snapshots)
            .map_err(|error| format!("command {index} {command:?}: {error}"))?;
    }
    Ok(())
}

struct ActualStep {
    accepted: bool,
    transition: Option<Transition>,
    submission: Option<Vec<FindingKey>>,
}

struct RetainedSnapshot {
    snapshot: SubmissionSnapshot,
    data: Value,
    revision: DataRevision,
    fingerprint: DefinitionFingerprint,
}

fn execute(
    form: &mut Form,
    definition: &FormDefinition,
    command: &Command,
    retained_snapshots: &mut Vec<RetainedSnapshot>,
) -> ActualStep {
    let result = match command {
        Command::InputName(text) => {
            let target = node(form, "/name");
            form.user().input_text(target, text)
        }
        Command::InputAmount(text) => {
            let target = node(form, "/amount");
            form.user().input_text(target, text)
        }
        Command::InputQuantity(text) => {
            let target = node(form, "/quantity");
            form.user().input_text(target, text)
        }
        Command::SetNullable(action) => {
            let target = node(form, "/nullable");
            match action {
                NullableCommand::Null => form.user().set_null(target),
                NullableCommand::Remove => form.user().remove_value(target),
                NullableCommand::Text => form.user().input_text(target, "repaired"),
                NullableCommand::Repair => form.user().replace_value(target, json!("repaired")),
            }
        }
        Command::MaterializeDetails => {
            let target = node(form, "/details");
            form.user().materialize(target)
        }
        Command::RemoveDetails => {
            let target = node(form, "/details");
            form.user().remove_value(target)
        }
        Command::InputDetail(text) => {
            let target = node(form, "/details/label");
            form.user().input_text(target, text)
        }
        Command::Blur(target) => {
            let binding = target.binding();
            let target = if matches!(target, Target::Root) {
                form.view().root()
            } else {
                node(form, binding)
            };
            form.user().blur(target)
        }
        Command::Host(action) => return execute_host(form, *action),
        Command::External {
            fresh,
            source,
            target,
            blocking,
        } => {
            let revision = if *fresh {
                form.view().data_revision()
            } else {
                definition
                    .create_form(json!({}))
                    .unwrap()
                    .view()
                    .data_revision()
            };
            let pointer = pointer(target.binding());
            let finding = if *blocking {
                ExternalFinding::blocking("host-finding", pointer, json!({}))
            } else {
                ExternalFinding::advisory("host-finding", pointer, json!({}))
            };
            match form.apply_external_findings(ExternalFindingBatch::new(
                source.name(),
                revision,
                [finding],
            )) {
                Ok(transition) => return accepted(transition),
                Err(_) => return rejected(),
            }
        }
        Command::Visibility {
            validation,
            external,
        } => Ok(form.set_finding_visibility(FindingVisibilityPolicy::new(
            (*validation).into(),
            (*external).into(),
        ))),
        Command::Reset => Ok(form.reset()),
        Command::Reinitialize(kind) => {
            let data = match kind {
                ReinitializeCommand::Valid => {
                    json!({ "amount": 1.5, "name": "Lin", "quantity": 7, "nullable": null })
                }
                ReinitializeCommand::Invalid => {
                    json!({ "amount": 2.5, "name": "", "quantity": 0 })
                }
                ReinitializeCommand::Equal => form.form_data().clone(),
                ReinitializeCommand::Rejected => json!([]),
            };
            match form.reinitialize(data) {
                Ok(transition) => return accepted(transition),
                Err(_) => return rejected(),
            }
        }
        Command::Submit => {
            let (transition, outcome) = form.prepare_submission().into_parts();
            let submission = match outcome {
                SubmissionOutcome::Ready(snapshot) => {
                    assert_eq!(snapshot.form_data(), form.form_data());
                    assert_eq!(snapshot.data_revision(), form.view().data_revision());
                    assert_eq!(
                        snapshot.definition_fingerprint(),
                        form.definition().fingerprint()
                    );
                    retained_snapshots.push(RetainedSnapshot {
                        data: snapshot.form_data().clone(),
                        revision: snapshot.data_revision(),
                        fingerprint: snapshot.definition_fingerprint(),
                        snapshot,
                    });
                    Vec::new()
                }
                SubmissionOutcome::Blocked(blockers) => blockers
                    .iter()
                    .map(|blocker| blocker_key(form, blocker))
                    .collect(),
            };
            return ActualStep {
                accepted: true,
                transition: Some(transition),
                submission: Some(submission),
            };
        }
    };
    match result {
        Ok(transition) => accepted(transition),
        Err(_) => rejected(),
    }
}

fn execute_host(form: &mut Form, action: HostCommand) -> ActualStep {
    let result = match action {
        HostCommand::ClosureFailed => match form.try_transact(|draft| {
            draft.set(&pointer("/name"), json!("discarded"));
            Err::<(), _>("closure-failed")
        }) {
            Err(TransactionError::Closure(_)) => return rejected(),
            _ => panic!("closure failure should reject atomically"),
        },
        HostCommand::InvalidOperation => form.transact(|draft| {
            draft.set(&pointer("/name"), json!("must-roll-back"));
            draft.remove(&pointer("/definitely-missing"));
        }),
        HostCommand::Commit => form.transact(|draft| {
            draft.set(&pointer("/name"), json!("Host"));
            draft.set(&pointer("/quantity"), json!(2));
        }),
        HostCommand::CommitInvalidData => form.transact(|draft| draft.remove(&pointer("/name"))),
        HostCommand::Repair => form.transact(|draft| {
            draft.set(&pointer("/name"), json!("Repaired"));
            draft.set(&pointer("/quantity"), json!(3));
        }),
        HostCommand::BreakNullable => {
            form.transact(|draft| draft.set(&pointer("/nullable"), json!(7)))
        }
        HostCommand::EquivalentQuantity => {
            form.transact(|draft| draft.set(&pointer("/quantity"), json!(1000)))
        }
    };
    match result {
        Ok(transition) => accepted(transition),
        Err(_) => rejected(),
    }
}

fn accepted(transition: Transition) -> ActualStep {
    ActualStep {
        accepted: true,
        transition: Some(transition),
        submission: None,
    }
}

fn rejected() -> ActualStep {
    ActualStep {
        accepted: false,
        transition: None,
        submission: None,
    }
}

fn assert_observation(form: &Form, model: &Model) -> Result<(), String> {
    if form.form_data() != &model.data {
        return Err(format!(
            "canonical data mismatch; expected {}, got {}",
            model.data,
            form.form_data()
        ));
    }
    for binding in BINDINGS {
        let view = form
            .node(node(form, binding))
            .expect("the fixed node should exist");
        let actual = NodeState {
            buffer: view.edit_buffer().map(str::to_owned),
            blocker: view.parse_blocker(),
            touched: view.is_touched(),
            dirty: view.is_dirty(),
        };
        let expected = model.node_state(binding);
        if actual != expected {
            return Err(format!(
                "{binding} state mismatch; expected {expected:?}, got {actual:?}"
            ));
        }
    }
    let validation = match form.view().validation_outcome() {
        ValidationOutcomeView::Valid => Vec::new(),
        ValidationOutcomeView::Invalid {
            findings,
            truncated: false,
        } => findings.iter().map(validation_finding_key).collect(),
        other => return Err(format!("unexpected validation outcome: {other:?}")),
    };
    if validation != model.validation_findings() {
        return Err(format!(
            "validation order mismatch; expected {:?}, got {validation:?}",
            model.validation_findings()
        ));
    }
    let visible = form
        .view()
        .visible_findings()
        .map(|finding| finding_key(form, finding))
        .collect::<Vec<_>>();
    if visible != model.visible_findings() {
        return Err(format!(
            "visible finding order mismatch; expected {:?}, got {visible:?}",
            model.visible_findings()
        ));
    }
    if form.view().submission_attempted() != model.submission_attempted {
        return Err("submission-attempt state mismatch".to_owned());
    }
    Ok(())
}

fn assert_retained_snapshots(snapshots: &[RetainedSnapshot]) -> Result<(), String> {
    for retained in snapshots {
        if retained.snapshot.form_data() != &retained.data
            || retained.snapshot.data_revision() != retained.revision
            || retained.snapshot.definition_fingerprint() != retained.fingerprint
        {
            return Err(
                "a retained submission snapshot changed after later form mutations".to_owned(),
            );
        }
    }
    Ok(())
}

fn fixed_identities(form: &Form) -> Vec<(&'static str, InstanceIdentity)> {
    BINDINGS
        .into_iter()
        .map(|binding| (binding, node(form, binding)))
        .collect()
}

fn assert_fixed_identities(
    form: &Form,
    root: InstanceIdentity,
    expected: &[(&str, InstanceIdentity)],
) -> Result<(), String> {
    if form.view().root() != root || form.node(root).is_none() {
        return Err("the fixed-object root identity changed".to_owned());
    }
    for (binding, identity) in expected {
        let current_binding = form
            .node(*identity)
            .and_then(|node| node.binding())
            .map(|binding| binding.pointer().as_str().to_owned());
        if current_binding.as_deref() != Some(*binding) {
            return Err(format!("the fixed identity for {binding} changed"));
        }
    }
    Ok(())
}

fn finding_key(form: &Form, finding: FindingView<'_>) -> FindingKey {
    match finding {
        FindingView::Parse { target, kind } => FindingKey::Parse(binding(form, target), kind),
        FindingView::Validation { finding, .. } => validation_finding_key(finding),
        FindingView::External {
            source, finding, ..
        } => FindingKey::External(
            source.to_owned(),
            finding.instance_location().as_str().to_owned(),
            finding.code().to_owned(),
            finding.is_blocking(),
            finding.parameters().clone(),
        ),
        FindingView::ValidationFindingsTruncated { .. }
        | FindingView::Indeterminate { .. }
        | FindingView::Capability { .. } => {
            panic!("the model schema cannot produce this finding family")
        }
        _ => panic!("the model does not recognize this finding family"),
    }
}

fn blocker_key(form: &Form, blocker: &SubmissionBlocker) -> FindingKey {
    match blocker {
        SubmissionBlocker::Parse { target, kind } => {
            FindingKey::Parse(binding(form, *target), *kind)
        }
        SubmissionBlocker::Validation(finding) => validation_finding_key(finding),
        SubmissionBlocker::External { source, finding } => FindingKey::External(
            source.clone(),
            finding.instance_location().as_str().to_owned(),
            finding.code().to_owned(),
            finding.is_blocking(),
            finding.parameters().clone(),
        ),
        SubmissionBlocker::ValidationFindingsTruncated { .. }
        | SubmissionBlocker::Indeterminate(_)
        | SubmissionBlocker::Capability(_) => {
            panic!("the model schema cannot produce this blocker family")
        }
        _ => panic!("the model does not recognize this blocker family"),
    }
}

fn validation_finding_key(finding: &schemaform::ValidationFinding) -> FindingKey {
    FindingKey::Validation {
        instance: finding.instance_location().as_str().to_owned(),
        resource: finding.keyword_location().resource().as_str().to_owned(),
        keyword: finding.keyword_location().pointer().as_str().to_owned(),
        code: finding.code().to_owned(),
        parameters: finding.parameters().clone(),
    }
}

fn definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["amount", "name", "quantity"],
        "properties": {
            "amount": { "type": "number" },
            "name": { "type": "string", "minLength": 2 },
            "quantity": { "type": "integer", "minimum": 1 },
            "nullable": { "type": ["string", "null"] },
            "details": {
                "type": "object",
                "additionalProperties": false,
                "properties": { "label": { "type": "string", "minLength": 2 } }
            }
        }
    }))
    .expect("the model schema should compile")
}

fn node(form: &Form, wanted: &str) -> InstanceIdentity {
    let mut pending = vec![form.view().root()];
    while let Some(identity) = pending.pop() {
        let view = form.node(identity).expect("the form node should exist");
        if view
            .binding()
            .is_some_and(|binding| binding.pointer().as_str() == wanted)
        {
            return identity;
        }
        pending.extend(view.children());
    }
    panic!("missing model binding {wanted}")
}

fn binding(form: &Form, identity: InstanceIdentity) -> String {
    form.node(identity)
        .and_then(|node| node.binding())
        .map(|binding| binding.pointer().as_str().to_owned())
        .unwrap_or_default()
}

fn pointer(value: &str) -> JsonPointer {
    JsonPointer::parse(value).expect("model pointers are valid")
}

fn set_member(data: &mut Value, member: &str, value: Value) {
    data.as_object_mut()
        .expect("model data remains an object")
        .insert(member.to_owned(), value);
}

fn remove_member(data: &mut Value, member: &str) {
    data.as_object_mut()
        .expect("model data remains an object")
        .remove(member);
}

fn exact_json(source: &str) -> Value {
    serde_json::from_str(source).expect("model JSON should parse")
}

struct ExactRational {
    numerator: BigInt,
    denominator: BigInt,
}

enum NumberParseFailure {
    Invalid,
    Expansion,
}

fn exact_rational(source: &str) -> Result<ExactRational, NumberParseFailure> {
    if !matches!(serde_json::from_str(source), Ok(Value::Number(_))) {
        return Err(NumberParseFailure::Invalid);
    }
    let (negative, unsigned) = source
        .strip_prefix('-')
        .map_or((false, source), |rest| (true, rest));
    let (significand, exponent) = unsigned.find(['e', 'E']).map_or((unsigned, "0"), |index| {
        (&unsigned[..index], &unsigned[index + 1..])
    });
    let exponent = exponent
        .parse::<i64>()
        .map_err(|_| NumberParseFailure::Expansion)?;
    let (whole, fraction) = significand.split_once('.').unwrap_or((significand, ""));
    let mut numerator = format!("{whole}{fraction}")
        .parse::<BigInt>()
        .map_err(|_| NumberParseFailure::Invalid)?;
    if negative {
        numerator = -numerator;
    }
    let scale = exponent
        .checked_sub(i64::try_from(fraction.len()).map_err(|_| NumberParseFailure::Expansion)?)
        .ok_or(NumberParseFailure::Expansion)?;
    let power = scale.unsigned_abs();
    if power > 10_000 {
        return Err(NumberParseFailure::Expansion);
    }
    let factor =
        BigInt::from(10_u8).pow(u32::try_from(power).map_err(|_| NumberParseFailure::Expansion)?);
    Ok(ExactRational {
        numerator: if scale >= 0 {
            numerator * factor.clone()
        } else {
            numerator
        },
        denominator: if scale >= 0 { 1.into() } else { factor },
    })
}

fn parse_integer(source: &str) -> Result<BigInt, ParseBlockerKind> {
    let rational = exact_rational(source).map_err(|failure| match failure {
        NumberParseFailure::Invalid => ParseBlockerKind::InvalidInteger,
        NumberParseFailure::Expansion => ParseBlockerKind::ResourceLimitExceeded,
    })?;
    if &rational.numerator % &rational.denominator != 0.into() {
        return Err(ParseBlockerKind::InvalidInteger);
    }
    let integer = rational.numerator / rational.denominator;
    if integer.to_string().trim_start_matches('-').len() > 4096 {
        return Err(ParseBlockerKind::ResourceLimitExceeded);
    }
    Ok(integer)
}

fn semantic_equal(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Number(left), Value::Number(right)) => {
            match (
                exact_rational(&left.to_string()),
                exact_rational(&right.to_string()),
            ) {
                (Ok(left), Ok(right)) => {
                    left.numerator * right.denominator == right.numerator * left.denominator
                }
                _ => left == right,
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| semantic_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, left)| {
                    right
                        .get(key)
                        .is_some_and(|right| semantic_equal(left, right))
                })
        }
        _ => left == right,
    }
}

fn optional_semantic_equal(left: Option<&Value>, right: Option<&Value>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => semantic_equal(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn preserve_semantically_equal(before: &Value, after: &mut Value) {
    if let (Some(before), Some(after)) = (before.as_object(), after.as_object_mut()) {
        for (key, candidate) in after.iter_mut() {
            if let Some(existing) = before.get(key)
                && semantic_equal(existing, candidate)
            {
                *candidate = existing.clone();
            }
        }
    }
}
