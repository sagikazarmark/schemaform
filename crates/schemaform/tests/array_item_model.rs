use std::collections::{HashMap, HashSet};

use schemaform::{
    DataRevision, DefinitionFingerprint, ExternalFinding, ExternalFindingBatch, FindingView, Form,
    FormDefinition, InstanceIdentity, ItemIdentity, JsonPointer, SubmissionOutcome,
    SubmissionSnapshot, Transition,
    form::{FindingVisibility, FindingVisibilityPolicy, ParseBlockerKind, SubmissionBlocker},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);

const MAX_ROWS: usize = 6;
const REPLAYS: [(&str, &str); 2] = [
    (
        "duplicate-state",
        include_str!("replays/array_item/duplicate-state.json"),
    ),
    (
        "lifecycle",
        include_str!("replays/array_item/lifecycle.json"),
    ),
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum Selector {
    First,
    Last,
    Slot(u8),
    Stale,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum Field {
    Name,
    Amount,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum Replacement {
    HostFirst,
    DifferentMiddle,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum Reinitialize {
    Valid,
    Invalid,
    Equal,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
enum Command {
    Append,
    InsertBefore(Selector),
    Remove(Selector),
    MoveUp(Selector),
    MoveDown(Selector),
    Input {
        target: Selector,
        field: Field,
        text: String,
    },
    Blur {
        target: Selector,
        field: Field,
    },
    Replace(Replacement),
    External {
        fresh: bool,
        target: Selector,
        field: Field,
        blocking: bool,
    },
    Reset,
    Reinitialize(Reinitialize),
    Submit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Row {
    id: u64,
    name: String,
    amount: i64,
    name_buffer: Option<String>,
    amount_buffer: Option<String>,
    amount_blocker: bool,
    name_touched: bool,
    amount_touched: bool,
}

impl Row {
    fn new(id: u64, name: &str, amount: i64) -> Self {
        Self {
            id,
            name: name.to_owned(),
            amount,
            name_buffer: None,
            amount_buffer: None,
            amount_blocker: false,
            name_touched: false,
            amount_touched: false,
        }
    }

    fn value(&self) -> Value {
        json!({ "name": self.name, "amount": self.amount })
    }

    fn clear_state(&mut self) {
        self.clear_edits();
        self.name_touched = false;
        self.amount_touched = false;
    }

    fn clear_edits(&mut self) {
        self.name_buffer = None;
        self.amount_buffer = None;
        self.amount_blocker = false;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExternalModel {
    row: u64,
    field: Field,
    blocking: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Model {
    rows: Vec<Row>,
    baseline: Vec<Row>,
    next_id: u64,
    external: Option<ExternalModel>,
    submission_attempted: bool,
}

#[derive(Debug)]
struct ModelStep {
    accepted: bool,
    force_revisions: bool,
    submission: Option<Vec<FindingKey>>,
}

impl ModelStep {
    fn accepted() -> Self {
        Self {
            accepted: true,
            force_revisions: false,
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

impl Model {
    fn new() -> Self {
        let rows = vec![Row::new(1, "same", 1), Row::new(2, "same", 1)];
        Self {
            baseline: rows.clone(),
            rows,
            next_id: 3,
            external: None,
            submission_attempted: false,
        }
    }

    fn data(&self) -> Value {
        json!({ "rows": self.rows.iter().map(Row::value).collect::<Vec<_>>() })
    }

    fn selected_index(&self, selector: Selector) -> Option<usize> {
        match selector {
            Selector::First => (!self.rows.is_empty()).then_some(0),
            Selector::Last => self.rows.len().checked_sub(1),
            Selector::Slot(slot) => {
                (!self.rows.is_empty()).then_some(usize::from(slot) % self.rows.len())
            }
            Selector::Stale => None,
        }
    }

    fn selected_id(&self, selector: Selector) -> Option<u64> {
        self.selected_index(selector)
            .map(|index| self.rows[index].id)
    }

    fn fresh_row(&mut self, name: &str, amount: i64) -> Row {
        let id = self.next_id;
        self.next_id += 1;
        Row::new(id, name, amount)
    }

    fn apply(&mut self, command: &Command) -> ModelStep {
        match command {
            Command::Append => {
                if self.rows.len() == MAX_ROWS {
                    return ModelStep::rejected();
                }
                let row = self.fresh_row("same", 1);
                self.rows.push(row);
                self.external = None;
                ModelStep::accepted()
            }
            Command::InsertBefore(selector) => {
                let Some(index) = self.selected_index(*selector) else {
                    return ModelStep::rejected();
                };
                if self.rows.len() == MAX_ROWS {
                    return ModelStep::rejected();
                }
                let row = self.fresh_row("same", 1);
                self.rows.insert(index, row);
                self.external = None;
                ModelStep::accepted()
            }
            Command::Remove(selector) => {
                let Some(index) = self.selected_index(*selector) else {
                    return ModelStep::rejected();
                };
                if self.rows.len() == 1 {
                    return ModelStep::rejected();
                }
                self.rows.remove(index);
                self.external = None;
                ModelStep::accepted()
            }
            Command::MoveUp(selector) => self.move_row(*selector, true),
            Command::MoveDown(selector) => self.move_row(*selector, false),
            Command::Input {
                target,
                field,
                text,
            } => {
                let Some(index) = self.selected_index(*target) else {
                    return ModelStep::rejected();
                };
                let row = &mut self.rows[index];
                let changed = match field {
                    Field::Name => {
                        row.name_buffer = Some(text.clone());
                        let changed = row.name != *text;
                        row.name.clone_from(text);
                        changed
                    }
                    Field::Amount => {
                        row.amount_buffer = Some(text.clone());
                        match text.parse::<i64>() {
                            Ok(value) => {
                                row.amount_blocker = false;
                                let changed = row.amount != value;
                                row.amount = value;
                                changed
                            }
                            Err(_) => {
                                row.amount_blocker = true;
                                false
                            }
                        }
                    }
                };
                if changed {
                    self.external = None;
                }
                ModelStep::accepted()
            }
            Command::Blur { target, field } => {
                let Some(index) = self.selected_index(*target) else {
                    return ModelStep::rejected();
                };
                let row = &mut self.rows[index];
                match field {
                    Field::Name => {
                        row.name_buffer = None;
                        row.name_touched = true;
                    }
                    Field::Amount => {
                        if !row.amount_blocker {
                            row.amount_buffer = None;
                        }
                        row.amount_touched = true;
                    }
                }
                ModelStep::accepted()
            }
            Command::Replace(replacement) => {
                let values = match replacement {
                    Replacement::HostFirst => [("host", 2), ("same", 1), ("same", 1)],
                    Replacement::DifferentMiddle => [("same", 1), ("other", 3), ("same", 1)],
                };
                let candidate = values
                    .into_iter()
                    .map(|(name, amount)| json!({ "name": name, "amount": amount }))
                    .collect::<Vec<_>>();
                if self.data()["rows"] == json!(candidate) {
                    for row in &mut self.rows {
                        row.clear_edits();
                    }
                    return ModelStep::accepted();
                }
                self.rows = values
                    .into_iter()
                    .map(|(name, amount)| self.fresh_row(name, amount))
                    .collect();
                self.external = None;
                ModelStep::accepted()
            }
            Command::External {
                fresh,
                target,
                field,
                blocking,
            } => {
                if !fresh {
                    return ModelStep::rejected();
                }
                // External findings are path-addressed, not item-identity-addressed.
                let row = self.selected_id(*target).unwrap_or_else(|| self.rows[0].id);
                self.external = Some(ExternalModel {
                    row,
                    field: *field,
                    blocking: *blocking,
                });
                ModelStep::accepted()
            }
            Command::Reset => {
                let current_values = self.rows.iter().map(Row::value).collect::<Vec<_>>();
                let baseline_values = self.baseline.iter().map(Row::value).collect::<Vec<_>>();
                let external = if current_values == baseline_values {
                    self.external.as_ref().and_then(|finding| {
                        if self.baseline.iter().any(|row| row.id == finding.row) {
                            Some(finding.clone())
                        } else {
                            self.rows
                                .iter()
                                .position(|row| row.id == finding.row)
                                .and_then(|index| self.baseline.get(index))
                                .map(|row| ExternalModel {
                                    row: row.id,
                                    field: finding.field,
                                    blocking: finding.blocking,
                                })
                        }
                    })
                } else {
                    None
                };
                self.rows = self.baseline.clone();
                for row in &mut self.rows {
                    row.clear_state();
                }
                self.external = external;
                self.submission_attempted = false;
                ModelStep::accepted()
            }
            Command::Reinitialize(kind) => {
                if matches!(kind, Reinitialize::Rejected) {
                    return ModelStep::rejected();
                }
                let values = match kind {
                    Reinitialize::Valid => vec![
                        ("same".to_owned(), 1),
                        ("same".to_owned(), 1),
                        ("ready".to_owned(), 2),
                    ],
                    Reinitialize::Invalid => {
                        vec![(String::new(), 0), ("same".to_owned(), 1)]
                    }
                    Reinitialize::Equal => self
                        .rows
                        .iter()
                        .map(|row| (row.name.clone(), row.amount))
                        .collect(),
                    Reinitialize::Rejected => unreachable!(),
                };
                self.rows = values
                    .into_iter()
                    .map(|(name, amount)| self.fresh_row(&name, amount))
                    .collect();
                self.baseline = self.rows.clone();
                self.external = None;
                self.submission_attempted = false;
                ModelStep {
                    force_revisions: true,
                    ..ModelStep::accepted()
                }
            }
            Command::Submit => {
                for row in &mut self.rows {
                    row.name_buffer = None;
                    if !row.amount_blocker {
                        row.amount_buffer = None;
                    }
                }
                self.submission_attempted = true;
                let mut step = ModelStep::accepted();
                step.submission = Some(self.submission_blockers());
                step
            }
        }
    }

    fn move_row(&mut self, selector: Selector, up: bool) -> ModelStep {
        let Some(index) = self.selected_index(selector) else {
            return ModelStep::rejected();
        };
        let other = if up {
            let Some(other) = index.checked_sub(1) else {
                return ModelStep::rejected();
            };
            other
        } else {
            let other = index + 1;
            if other == self.rows.len() {
                return ModelStep::rejected();
            }
            other
        };
        let data_changed = self.rows[index].value() != self.rows[other].value();
        self.rows.swap(index, other);
        if data_changed {
            self.external = None;
        }
        ModelStep::accepted()
    }

    fn validation_findings(&self) -> Vec<FindingKey> {
        let mut findings = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            if row.name.chars().count() < 2 {
                findings.push(validation_key(
                    &format!("/rows/{index}/name"),
                    "/properties/rows/items/properties/name/minLength",
                    "minLength",
                    json!({ "limit": 2 }),
                ));
            }
            if row.amount < 1 {
                findings.push(validation_key(
                    &format!("/rows/{index}/amount"),
                    "/properties/rows/items/properties/amount/minimum",
                    "minimum",
                    json!({ "limit": 1 }),
                ));
            }
        }
        findings.sort_by(|left, right| finding_sort_key(left).cmp(&finding_sort_key(right)));
        findings
    }

    fn parse_findings(&self) -> Vec<FindingKey> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row.amount_blocker)
            .map(|(index, _)| {
                FindingKey::Parse(
                    format!("/rows/{index}/amount"),
                    ParseBlockerKind::InvalidInteger,
                )
            })
            .collect()
    }

    fn external_finding(&self) -> Option<FindingKey> {
        let external = self.external.as_ref()?;
        let index = self.rows.iter().position(|row| row.id == external.row)?;
        Some(FindingKey::External {
            source: "server".to_owned(),
            instance: format!("/rows/{index}/{}", field_name(external.field)),
            code: "host".to_owned(),
            blocking: external.blocking,
            parameters: json!({}),
        })
    }

    fn visible_findings(&self) -> Vec<FindingKey> {
        let mut findings = self.validation_findings();
        findings.extend(self.external_finding());
        findings.extend(self.parse_findings());
        findings
    }

    fn submission_blockers(&self) -> Vec<FindingKey> {
        let mut blockers = self.parse_findings();
        blockers.extend(self.validation_findings());
        if self
            .external
            .as_ref()
            .is_some_and(|finding| finding.blocking)
        {
            blockers.extend(self.external_finding());
        }
        blockers
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FindingKey {
    Parse(String, ParseBlockerKind),
    Validation {
        instance: String,
        resource: String,
        keyword: String,
        code: String,
        parameters: Value,
    },
    External {
        source: String,
        instance: String,
        code: String,
        blocking: bool,
        parameters: Value,
    },
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

fn finding_sort_key(finding: &FindingKey) -> (&str, &str, &str, &str) {
    match finding {
        FindingKey::Validation {
            instance,
            resource,
            keyword,
            code,
            ..
        } => (instance, resource, keyword, code),
        FindingKey::Parse(instance, _) => (instance, "", "", ""),
        FindingKey::External {
            instance,
            source,
            code,
            ..
        } => (instance, source, "", code),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PublicTopology {
    row: InstanceIdentity,
    item: ItemIdentity,
    name: InstanceIdentity,
    amount: InstanceIdentity,
    subtree: Vec<InstanceIdentity>,
}

struct IdentityRegistry {
    by_model: HashMap<u64, PublicTopology>,
    ever_seen: HashSet<InstanceIdentity>,
    stale: PublicTopology,
}

impl IdentityRegistry {
    fn new(form: &mut Form, array: InstanceIdentity, model: &Model) -> Self {
        // Create one retired identity up front so stale-target commands are real facade calls.
        form.user().append_item(array).unwrap();
        let stale = public_rows(form, array).pop().unwrap();
        form.user().remove_item(array, stale.item).unwrap();
        form.reset();

        let current = public_rows(form, array);
        let mut registry = Self {
            by_model: HashMap::new(),
            ever_seen: stale.subtree.iter().copied().collect(),
            stale,
        };
        for (row, topology) in model.rows.iter().zip(current) {
            registry.record(row.id, topology).unwrap();
        }
        registry
    }

    fn record(&mut self, id: u64, topology: PublicTopology) -> Result<(), String> {
        if topology
            .subtree
            .iter()
            .any(|identity| self.ever_seen.contains(identity))
        {
            return Err(format!(
                "fresh logical row {id} aliases a current or retired subtree identity"
            ));
        }
        self.ever_seen.extend(topology.subtree.iter().copied());
        self.by_model.insert(id, topology);
        Ok(())
    }

    fn target_topology(&self, model: &Model, selector: Selector) -> &PublicTopology {
        model
            .selected_id(selector)
            .and_then(|id| self.by_model.get(&id))
            .unwrap_or(&self.stale)
    }

    fn sync(&mut self, form: &Form, array: InstanceIdentity, model: &Model) -> Result<(), String> {
        let actual = public_rows(form, array);
        if actual.len() != model.rows.len() {
            return Err(format!(
                "row count mismatch: expected {}, got {}",
                model.rows.len(),
                actual.len()
            ));
        }
        for (row, topology) in model.rows.iter().zip(actual) {
            if let Some(expected) = self.by_model.get(&row.id) {
                if expected != &topology {
                    return Err(format!(
                        "logical row {} did not retain its complete public identity topology",
                        row.id
                    ));
                }
            } else {
                self.record(row.id, topology)?;
            }
        }
        let current = model.rows.iter().map(|row| row.id).collect::<HashSet<_>>();
        for (id, topology) in &self.by_model {
            if !current.contains(id)
                && topology
                    .subtree
                    .iter()
                    .any(|identity| form.node(*identity).is_some())
            {
                return Err(format!("retired logical row {id} remains addressable"));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
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

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn checked_in_focused_replays_match_the_array_item_model() {
    for (name, source) in REPLAYS {
        let trace = serde_json::from_str::<Vec<Command>>(source).unwrap();
        run_trace(&trace).unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
#[ignore = "expensive deterministic soak run explicitly"]
fn retained_deterministic_1024_command_soak_matches_the_array_item_model() {
    run_trace(&soak_trace()).unwrap();
}

fn soak_trace() -> Vec<Command> {
    let cycle = [
        Command::Append,
        Command::Input {
            target: Selector::Last,
            field: Field::Name,
            text: "same".to_owned(),
        },
        Command::MoveUp(Selector::Last),
        Command::MoveDown(Selector::First),
        Command::Input {
            target: Selector::Last,
            field: Field::Amount,
            text: "-".to_owned(),
        },
        Command::Blur {
            target: Selector::Last,
            field: Field::Amount,
        },
        Command::Submit,
        Command::Input {
            target: Selector::Last,
            field: Field::Amount,
            text: "1".to_owned(),
        },
        Command::External {
            fresh: true,
            target: Selector::Last,
            field: Field::Name,
            blocking: true,
        },
        Command::InsertBefore(Selector::First),
        Command::Remove(Selector::Last),
        Command::MoveUp(Selector::First),
        Command::External {
            fresh: false,
            target: Selector::First,
            field: Field::Name,
            blocking: false,
        },
        Command::Replace(Replacement::HostFirst),
        Command::Reset,
        Command::Reinitialize(Reinitialize::Equal),
    ];
    let trace = cycle.into_iter().cycle().take(1_024).collect::<Vec<_>>();
    assert_eq!(trace.len(), 1_024);
    trace
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
#[test]
#[ignore = "expensive property test run explicitly in CI"]
fn generated_array_item_traces_match_the_reference_model() {
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
    let definition = definition();
    run_trace_with_definition(&definition, &soak_trace())
        .expect("the retained 1,024-command soak should match the model");
    let config = Config {
        cases,
        rng_seed: RngSeed::Fixed(0x5eed_a229_2026_0722),
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/replays/array_item/proptest-seeds.txt",
        ))),
        ..Config::default()
    };
    let mut runner = TestRunner::new(config);
    runner
        .run(
            &proptest::collection::vec(command_strategy(), 1..=256),
            |trace| {
                run_trace_with_definition(&definition, &trace).map_err(|error| {
                    TestCaseError::fail(format!(
                        "{error}\nminimized trace can be promoted as JSON:\n{}",
                        serde_json::to_string_pretty(&trace).unwrap()
                    ))
                })
            },
        )
        .unwrap();
}

#[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
fn command_strategy() -> impl proptest::strategy::Strategy<Value = Command> {
    use proptest::prelude::*;

    let selector = prop_oneof![
        Just(Selector::First),
        Just(Selector::Last),
        (0_u8..12).prop_map(Selector::Slot),
        Just(Selector::Stale),
    ];
    let field = prop_oneof![Just(Field::Name), Just(Field::Amount)];
    prop_oneof![
        4 => Just(Command::Append),
        4 => selector.clone().prop_map(Command::InsertBefore),
        5 => selector.clone().prop_map(Command::Remove),
        4 => selector.clone().prop_map(Command::MoveUp),
        4 => selector.clone().prop_map(Command::MoveDown),
        8 => (
            selector.clone(),
            field.clone(),
            prop_oneof![Just(""), Just("x"), Just("same"), Just("other"), Just("-"), Just("0"), Just("1"), Just("2")],
        ).prop_map(|(target, field, text)| Command::Input { target, field, text: text.to_owned() }),
        3 => (selector.clone(), field.clone()).prop_map(|(target, field)| Command::Blur { target, field }),
        3 => prop_oneof![Just(Replacement::HostFirst), Just(Replacement::DifferentMiddle)].prop_map(Command::Replace),
        5 => (any::<bool>(), selector, field, any::<bool>()).prop_map(|(fresh, target, field, blocking)| Command::External { fresh, target, field, blocking }),
        2 => Just(Command::Reset),
        3 => prop_oneof![Just(Reinitialize::Valid), Just(Reinitialize::Invalid), Just(Reinitialize::Equal), Just(Reinitialize::Rejected)].prop_map(Command::Reinitialize),
        3 => Just(Command::Submit),
    ]
}

fn run_trace(trace: &[Command]) -> Result<(), String> {
    run_trace_with_definition(&definition(), trace)
}

fn run_trace_with_definition(definition: &FormDefinition, trace: &[Command]) -> Result<(), String> {
    let mut model = Model::new();
    let mut form = definition
        .form(model.data())
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::Immediate,
        ))
        .build()
        .map_err(|error| error.to_string())?;
    let array = node(&form, "/rows");
    let mut identities = IdentityRegistry::new(&mut form, array, &model);
    let mut retained = Vec::new();
    assert_observation(&form, array, &model, &identities)?;

    for (index, command) in trace.iter().enumerate() {
        let before_model = model.clone();
        let before_data = form.view().data_revision();
        let before_state = form.view().state_revision();
        let expected = model.apply(command);
        let actual = execute(
            &mut form,
            definition,
            array,
            &before_model,
            &identities,
            command,
            &mut retained,
        );
        let context = || format!("command {index} {command:?}");
        if actual.accepted != expected.accepted {
            return Err(format!(
                "{}: acceptance mismatch, expected {}, got {}",
                context(),
                expected.accepted,
                actual.accepted
            ));
        }
        if let Some(transition) = &actual.transition {
            if transition.before_data_revision() != before_data
                || transition.before_state_revision() != before_state
                || transition.after_data_revision() != form.view().data_revision()
                || transition.after_state_revision() != form.view().state_revision()
            {
                return Err(format!(
                    "{}: transition does not bracket revisions",
                    context()
                ));
            }
            let expected_data = expected.force_revisions || before_model.data() != model.data();
            let expected_state = expected.force_revisions || before_model != model;
            if (before_data != transition.after_data_revision()) != expected_data
                || (before_state != transition.after_state_revision()) != expected_state
            {
                return Err(format!(
                    "{}: revision delta mismatch, expected data={expected_data}, state={expected_state}",
                    context()
                ));
            }
        } else if form.view().data_revision() != before_data
            || form.view().state_revision() != before_state
        {
            return Err(format!("{}: rejected command changed revisions", context()));
        }
        if let Some(transition) = &actual.transition {
            assert_transition_topology(
                &form,
                array,
                transition,
                &before_model,
                &model,
                &identities,
            )
            .map_err(|error| format!("{}: {error}", context()))?;
        }
        identities
            .sync(&form, array, &model)
            .map_err(|error| format!("{}: {error}", context()))?;
        if let Some(expected_submission) = expected.submission {
            if actual.submission != Some(expected_submission) {
                return Err(format!("{}: submission outcome mismatch", context()));
            }
        }
        assert_observation(&form, array, &model, &identities)
            .map_err(|error| format!("{}: {error}", context()))?;
        assert_retained_snapshots(&retained).map_err(|error| format!("{}: {error}", context()))?;
    }
    Ok(())
}

fn assert_transition_topology(
    form: &Form,
    array: InstanceIdentity,
    transition: &Transition,
    before: &Model,
    after: &Model,
    identities: &IdentityRegistry,
) -> Result<(), String> {
    let before_ids = before.rows.iter().map(|row| row.id).collect::<HashSet<_>>();
    let after_ids = after.rows.iter().map(|row| row.id).collect::<HashSet<_>>();
    let expected_removed = before_ids
        .difference(&after_ids)
        .flat_map(|id| identities.by_model[id].subtree.iter().copied())
        .collect::<HashSet<_>>();
    let removed = transition.removed().collect::<Vec<_>>();
    let removed_set = removed.iter().copied().collect::<HashSet<_>>();
    if removed.len() != removed_set.len() || removed_set != expected_removed {
        return Err("transition removed identities do not match retired row subtrees".to_owned());
    }

    let changed = transition.changed().collect::<Vec<_>>();
    let changed_set = changed.iter().copied().collect::<HashSet<_>>();
    if changed.len() != changed_set.len()
        || changed
            .iter()
            .any(|identity| form.node(*identity).is_none())
    {
        return Err("transition contains duplicate or stale changed identities".to_owned());
    }

    let actual = public_rows(form, array);
    for (after_index, (row, topology)) in after.rows.iter().zip(actual).enumerate() {
        let before_index = before
            .rows
            .iter()
            .position(|candidate| candidate.id == row.id);
        if before_index != Some(after_index) {
            for identity in topology.subtree {
                if !changed_set.contains(&identity) {
                    return Err(format!(
                        "transition omitted changed identity for new or reindexed logical row {}",
                        row.id
                    ));
                }
            }
        }
    }
    Ok(())
}

fn execute(
    form: &mut Form,
    definition: &FormDefinition,
    array: InstanceIdentity,
    model: &Model,
    identities: &IdentityRegistry,
    command: &Command,
    retained: &mut Vec<RetainedSnapshot>,
) -> ActualStep {
    let result = match command {
        Command::Append => form.user().append_item(array),
        Command::InsertBefore(selector) => form
            .user()
            .insert_item_before(array, identities.target_topology(model, *selector).item),
        Command::Remove(selector) => form
            .user()
            .remove_item(array, identities.target_topology(model, *selector).item),
        Command::MoveUp(selector) => form
            .user()
            .move_item_up(array, identities.target_topology(model, *selector).item),
        Command::MoveDown(selector) => form
            .user()
            .move_item_down(array, identities.target_topology(model, *selector).item),
        Command::Input {
            target,
            field,
            text,
        } => {
            let topology = identities.target_topology(model, *target);
            form.user()
                .input_text(field_identity(topology, *field), text)
        }
        Command::Blur { target, field } => {
            let topology = identities.target_topology(model, *target);
            form.user().blur(field_identity(topology, *field))
        }
        Command::Replace(replacement) => {
            let value = match replacement {
                Replacement::HostFirst => json!([
                    { "name": "host", "amount": 2 },
                    { "name": "same", "amount": 1 },
                    { "name": "same", "amount": 1 }
                ]),
                Replacement::DifferentMiddle => json!([
                    { "name": "same", "amount": 1 },
                    { "name": "other", "amount": 3 },
                    { "name": "same", "amount": 1 }
                ]),
            };
            return match form.transact(|draft| draft.set(&pointer("/rows"), value)) {
                Ok(transition) => accepted(transition),
                Err(_) => rejected(),
            };
        }
        Command::External {
            fresh,
            target,
            field,
            blocking,
        } => {
            let revision = if *fresh {
                form.view().data_revision()
            } else {
                definition
                    .create_form(json!({ "rows": [] }))
                    .unwrap()
                    .view()
                    .data_revision()
            };
            let path = model.selected_index(*target).map_or_else(
                || format!("/rows/0/{}", field_name(*field)),
                |index| format!("/rows/{index}/{}", field_name(*field)),
            );
            let finding = if *blocking {
                ExternalFinding::blocking("host", pointer(&path), json!({}))
            } else {
                ExternalFinding::advisory("host", pointer(&path), json!({}))
            };
            return match form.apply_external_findings(ExternalFindingBatch::new(
                "server",
                revision,
                [finding],
            )) {
                Ok(transition) => accepted(transition),
                Err(_) => rejected(),
            };
        }
        Command::Reset => Ok(form.reset()),
        Command::Reinitialize(kind) => {
            let data = match kind {
                Reinitialize::Valid => json!({
                    "rows": [
                        { "name": "same", "amount": 1 },
                        { "name": "same", "amount": 1 },
                        { "name": "ready", "amount": 2 }
                    ]
                }),
                Reinitialize::Invalid => json!({
                    "rows": [{ "name": "", "amount": 0 }, { "name": "same", "amount": 1 }]
                }),
                Reinitialize::Equal => form.form_data().clone(),
                Reinitialize::Rejected => json!([]),
            };
            return match form.reinitialize(data) {
                Ok(transition) => accepted(transition),
                Err(_) => rejected(),
            };
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
                    retained.push(RetainedSnapshot {
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
                    .collect::<Vec<_>>(),
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

fn assert_observation(
    form: &Form,
    array: InstanceIdentity,
    model: &Model,
    identities: &IdentityRegistry,
) -> Result<(), String> {
    if form.form_data() != &model.data() {
        return Err(format!(
            "canonical data mismatch: expected {}, got {}",
            model.data(),
            form.form_data()
        ));
    }
    if form.view().submission_attempted() != model.submission_attempted {
        return Err("submission-attempt state mismatch".to_owned());
    }
    let baseline_values = model.baseline.iter().map(Row::value).collect::<Vec<_>>();
    for (index, row) in model.rows.iter().enumerate() {
        let topology = identities.by_model.get(&row.id).unwrap();
        let row_path = format!("/rows/{index}");
        let name_path = format!("{row_path}/name");
        let amount_path = format!("{row_path}/amount");
        assert_binding(form, topology.row, &row_path)?;
        assert_binding(form, topology.name, &name_path)?;
        assert_binding(form, topology.amount, &amount_path)?;
        if form.node(topology.row).unwrap().current_data() != Some(&row.value()) {
            return Err(format!("logical row {} current data mismatch", row.id));
        }
        let name = form.node(topology.name).unwrap();
        let amount = form.node(topology.amount).unwrap();
        if name.edit_buffer() != row.name_buffer.as_deref()
            || name.parse_blocker().is_some()
            || name.is_touched() != row.name_touched
            || amount.edit_buffer() != row.amount_buffer.as_deref()
            || amount.parse_blocker()
                != row
                    .amount_blocker
                    .then_some(ParseBlockerKind::InvalidInteger)
            || amount.is_touched() != row.amount_touched
        {
            return Err(format!("logical row {} local state mismatch", row.id));
        }
        let baseline = baseline_values.get(index);
        let expected_row_dirty = baseline != Some(&row.value());
        let expected_name_dirty =
            baseline.and_then(|value| value.get("name")) != Some(&json!(row.name));
        let expected_amount_dirty =
            baseline.and_then(|value| value.get("amount")) != Some(&json!(row.amount));
        if form.node(topology.row).unwrap().is_dirty() != expected_row_dirty
            || name.is_dirty() != expected_name_dirty
            || amount.is_dirty() != expected_amount_dirty
        {
            return Err(format!("logical row {} dirty state mismatch", row.id));
        }
    }
    if form.node(array).unwrap().is_dirty() != (model.data()["rows"] != json!(baseline_values)) {
        return Err("array dirty state mismatch".to_owned());
    }
    let visible = form
        .view()
        .visible_findings()
        .map(|finding| finding_key(form, finding))
        .collect::<Vec<_>>();
    if visible != model.visible_findings() {
        return Err(format!(
            "finding mismatch: expected {:?}, got {visible:?}",
            model.visible_findings()
        ));
    }
    Ok(())
}

fn assert_retained_snapshots(snapshots: &[RetainedSnapshot]) -> Result<(), String> {
    for retained in snapshots {
        if retained.snapshot.form_data() != &retained.data
            || retained.snapshot.data_revision() != retained.revision
            || retained.snapshot.definition_fingerprint() != retained.fingerprint
        {
            return Err("a retained ready snapshot changed after a later mutation".to_owned());
        }
    }
    Ok(())
}

fn finding_key(form: &Form, finding: FindingView<'_>) -> FindingKey {
    match finding {
        FindingView::Parse { target, kind } => FindingKey::Parse(binding(form, target), kind),
        FindingView::Validation {
            target, finding, ..
        } => {
            assert_eq!(
                target,
                node(form, finding.instance_location().as_str()),
                "validation finding target must be the current repeated-node identity"
            );
            FindingKey::Validation {
                instance: finding.instance_location().as_str().to_owned(),
                resource: finding.keyword_location().resource().as_str().to_owned(),
                keyword: finding.keyword_location().pointer().as_str().to_owned(),
                code: finding.code().to_owned(),
                parameters: finding.parameters().clone(),
            }
        }
        FindingView::External {
            target,
            source,
            finding,
        } => {
            assert_eq!(
                target,
                node(form, finding.instance_location().as_str()),
                "external finding target must be the current repeated-node identity"
            );
            FindingKey::External {
                source: source.to_owned(),
                instance: finding.instance_location().as_str().to_owned(),
                code: finding.code().to_owned(),
                blocking: finding.is_blocking(),
                parameters: finding.parameters().clone(),
            }
        }
        FindingView::ValidationFindingsTruncated { .. }
        | FindingView::Indeterminate { .. }
        | FindingView::Capability { .. } => panic!("unexpected finding family in array model"),
        _ => panic!("the array model does not recognize this finding family"),
    }
}

fn blocker_key(form: &Form, blocker: &SubmissionBlocker) -> FindingKey {
    match blocker {
        SubmissionBlocker::Parse { target, kind } => {
            FindingKey::Parse(binding(form, *target), *kind)
        }
        SubmissionBlocker::Validation(finding) => FindingKey::Validation {
            instance: finding.instance_location().as_str().to_owned(),
            resource: finding.keyword_location().resource().as_str().to_owned(),
            keyword: finding.keyword_location().pointer().as_str().to_owned(),
            code: finding.code().to_owned(),
            parameters: finding.parameters().clone(),
        },
        SubmissionBlocker::External { source, finding } => FindingKey::External {
            source: source.clone(),
            instance: finding.instance_location().as_str().to_owned(),
            code: finding.code().to_owned(),
            blocking: finding.is_blocking(),
            parameters: finding.parameters().clone(),
        },
        SubmissionBlocker::ValidationFindingsTruncated { .. }
        | SubmissionBlocker::Indeterminate(_)
        | SubmissionBlocker::Capability(_) => {
            panic!("unexpected submission blocker in array model")
        }
        _ => panic!("the array model does not recognize this blocker family"),
    }
}

fn public_rows(form: &Form, array: InstanceIdentity) -> Vec<PublicTopology> {
    form.node(array)
        .unwrap()
        .children()
        .map(|row| {
            let binding = form.node(row).unwrap().binding().unwrap();
            let item = binding.item().unwrap();
            let row_path = binding.pointer().as_str();
            PublicTopology {
                row,
                item,
                name: descendant(form, row, &format!("{row_path}/name")),
                amount: descendant(form, row, &format!("{row_path}/amount")),
                subtree: subtree(form, row),
            }
        })
        .collect()
}

fn subtree(form: &Form, root: InstanceIdentity) -> Vec<InstanceIdentity> {
    let mut result = Vec::new();
    let mut pending = vec![root];
    while let Some(identity) = pending.pop() {
        result.push(identity);
        pending.extend(form.node(identity).unwrap().children());
    }
    result.sort_by_key(|identity| binding(form, *identity));
    result
}

fn node(form: &Form, wanted: &str) -> InstanceIdentity {
    descendant(form, form.view().root(), wanted)
}

fn descendant(form: &Form, root: InstanceIdentity, wanted: &str) -> InstanceIdentity {
    let mut pending = vec![root];
    while let Some(identity) = pending.pop() {
        let view = form.node(identity).unwrap();
        if view
            .binding()
            .is_some_and(|binding| binding.pointer().as_str() == wanted)
        {
            return identity;
        }
        pending.extend(view.children());
    }
    panic!("missing binding {wanted}")
}

fn assert_binding(form: &Form, identity: InstanceIdentity, expected: &str) -> Result<(), String> {
    let actual = form
        .node(identity)
        .and_then(|node| node.binding())
        .map(|binding| binding.pointer().as_str().to_owned());
    if actual.as_deref() != Some(expected) {
        return Err(format!(
            "identity binding mismatch: expected {expected}, got {actual:?}"
        ));
    }
    Ok(())
}

fn binding(form: &Form, identity: InstanceIdentity) -> String {
    form.node(identity)
        .and_then(|node| node.binding())
        .map(|binding| binding.pointer().as_str().to_owned())
        .unwrap_or_default()
}

fn field_identity(topology: &PublicTopology, field: Field) -> InstanceIdentity {
    match field {
        Field::Name => topology.name,
        Field::Amount => topology.amount,
    }
}

fn field_name(field: Field) -> &'static str {
    match field {
        Field::Name => "name",
        Field::Amount => "amount",
    }
}

fn pointer(value: &str) -> JsonPointer {
    JsonPointer::parse(value).unwrap()
}

fn definition() -> FormDefinition {
    FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["rows"],
        "properties": {
            "rows": {
                "type": "array",
                "minItems": 1,
                "maxItems": MAX_ROWS,
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "default": { "name": "same", "amount": 1 },
                    "required": ["name", "amount"],
                    "properties": {
                        "name": { "type": "string", "minLength": 2 },
                        "amount": { "type": "integer", "minimum": 1 }
                    }
                }
            }
        }
    }))
    .unwrap()
}
