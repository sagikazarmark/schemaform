use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashMap, HashSet},
    num::NonZeroUsize,
};

use schemaform::{
    ExternalFinding, ExternalFindingBatch, FindingView, FindingVisibility, FindingVisibilityPolicy,
    Form, FormDataLimits, FormDefinition, InstanceIdentity, ItemIdentity, JsonPointer, Transition,
    definition::SemanticKind,
    form::{
        DataRevision, ExternalFindingError, ExternalFindingLimits, HostCommitError,
        TransactionError, UserOperationError, ValidationOutcomeView,
    },
};
use serde::Serialize;
use serde_json::{Value, json};

pub const MAX_RUNTIME_COMMANDS: usize = 64;
pub const MAX_TRANSACTION_OPERATIONS: usize = 8;
pub const MAX_FINDINGS_PER_BATCH: usize = 17;
const COMMAND_BYTES: usize = 12;
const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";
const MAX_ACTIVE_EXTERNAL_FINDINGS: usize = 4;
const MAX_ACTIVE_EXTERNAL_FINDING_BYTES: usize = 128;
const MAX_INCOMING_EXTERNAL_FINDINGS: usize = MAX_ACTIVE_EXTERNAL_FINDINGS * 4;
const MAX_INCOMING_EXTERNAL_FINDING_BYTES: usize = MAX_ACTIVE_EXTERNAL_FINDING_BYTES * 4;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuntimeOutcome {
    steps: Vec<StepOutcome>,
    final_snapshot: NormalizedSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct StepOutcome {
    result: &'static str,
    data_changed: bool,
    state_changed: bool,
    changed: Vec<usize>,
    removed: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct NormalizedSnapshot {
    data: Value,
    validation: String,
    submission_attempted: bool,
    findings: Vec<String>,
    nodes: Vec<NormalizedNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct NormalizedNode {
    identity: usize,
    item: Option<usize>,
    kind: String,
    semantic_kind: String,
    binding: Option<String>,
    allowed: String,
    edit_buffer: Option<String>,
    parse_blocker: Option<String>,
    touched: bool,
    dirty: bool,
    current_data: Option<Value>,
    display_text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservableSnapshot {
    data: Value,
    data_revision: DataRevision,
    state_revision: schemaform::form::StateRevision,
    normalized: NormalizedSnapshot,
}

#[derive(Clone, Copy)]
struct Command([u8; COMMAND_BYTES]);

struct Runtime {
    form: Form,
    foreign_form: Form,
    identities: HashMap<InstanceIdentity, usize>,
    items: HashMap<ItemIdentity, usize>,
    retired: HashSet<InstanceIdentity>,
    retired_items: HashSet<ItemIdentity>,
    next_identity: usize,
    next_item: usize,
    stale_revision: Option<DataRevision>,
    external_model: BTreeMap<ExternalSource, Vec<ExpectedExternalFinding>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ExpectedExternalFinding {
    location: ExternalLocation,
    code: ExternalCode,
    blocking: bool,
    parameters: ExternalParameters,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalSource {
    Policy,
    Server,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalLocation {
    Name,
    Count,
    RowValue,
    LongUnmatched,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalCode {
    Finding0,
    Finding1,
    Finding2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExternalParameters {
    Zero,
    One,
}

#[derive(Clone, Copy)]
enum MovementDirection {
    Up,
    Down,
}

struct MovementExpectation {
    array: InstanceIdentity,
    item: ItemIdentity,
    destination: usize,
    subtree: HashSet<InstanceIdentity>,
}

pub fn user_commands(input: &[u8]) -> RuntimeOutcome {
    run_twice(input, Program::User)
}

pub fn host_transactions(input: &[u8]) -> RuntimeOutcome {
    run_twice(input, Program::Host)
}

pub fn external_findings(input: &[u8]) -> RuntimeOutcome {
    run_twice(input, Program::External)
}

#[derive(Clone, Copy)]
enum Program {
    User,
    Host,
    External,
}

fn run_twice(input: &[u8], program: Program) -> RuntimeOutcome {
    let commands = decode(input);
    let first = execute(&commands, program);
    let second = execute(&commands, program);
    assert_eq!(
        first, second,
        "runtime program produced a nondeterministic outcome"
    );
    first
}

fn decode(input: &[u8]) -> Vec<Command> {
    input
        .chunks(COMMAND_BYTES)
        .take(MAX_RUNTIME_COMMANDS)
        .map(|chunk| {
            let mut bytes = [0; COMMAND_BYTES];
            bytes[..chunk.len()].copy_from_slice(chunk);
            Command(bytes)
        })
        .collect()
}

fn execute(commands: &[Command], program: Program) -> RuntimeOutcome {
    let mut runtime = Runtime::new(program);
    let mut steps = Vec::with_capacity(commands.len());
    for command in commands {
        let step = match program {
            Program::User => runtime.user_command(*command),
            Program::Host => runtime.host_transaction(*command),
            Program::External => runtime.external_batch(*command),
        };
        steps.push(step);
    }
    let final_snapshot = runtime.normalized_snapshot();
    RuntimeOutcome {
        steps,
        final_snapshot,
    }
}

impl Runtime {
    fn new(program: Program) -> Self {
        let mut form = build_form();
        let foreign_form = build_form();
        let stale_revision = if matches!(program, Program::External) {
            let stale = form.view().data_revision();
            form.transact(|draft| draft.set(&pointer("/name"), json!("Primed")))
                .expect("reviewed priming transaction succeeds");
            Some(stale)
        } else {
            None
        };
        let mut runtime = Self {
            form,
            foreign_form,
            identities: HashMap::new(),
            items: HashMap::new(),
            retired: HashSet::new(),
            retired_items: HashSet::new(),
            next_identity: 0,
            next_item: 0,
            stale_revision,
            external_model: BTreeMap::new(),
        };
        runtime.observe_identity_lifecycle(&HashSet::new());
        runtime
    }

    fn user_command(&mut self, command: Command) -> StepOutcome {
        let before = self.observable_snapshot();
        let current_before = self
            .current_identities()
            .into_iter()
            .collect::<HashSet<_>>();
        let opcode = command.0[0] % 12;
        let target = self.select_identity(command.0[1], opcode >= 7);
        let item = self.select_item(command.0[2]);
        let movement = match opcode {
            10 => self.capture_movement(target, item, MovementDirection::Up),
            11 => self.capture_movement(target, item, MovementDirection::Down),
            _ => None,
        };
        let result = match opcode {
            0 => self
                .form
                .user()
                .input_text(target, text_value(command.0[3])),
            1 => self.form.user().blur(target),
            2 => self
                .form
                .user()
                .set_value(target, scalar_value(command.0[3])),
            3 => self.form.user().set_null(target),
            4 => self.form.user().remove_value(target),
            5 => self
                .form
                .user()
                .replace_value(target, replacement_value(command.0[3])),
            6 => self.form.user().materialize(target),
            7 => self.form.user().append_item(target),
            8 => self.form.user().insert_item_before(target, item),
            9 => self.form.user().remove_item(target, item),
            10 => self.form.user().move_item_up(target, item),
            _ => self.form.user().move_item_down(target, item),
        };
        match result {
            Ok(transition) => {
                if let Some(movement) = movement {
                    self.assert_movement(&movement);
                }
                self.accepted(before, current_before, transition, "accepted")
            }
            Err(error) => {
                self.rejected(&before);
                StepOutcome::rejected(user_error_kind(&error))
            }
        }
    }

    fn host_transaction(&mut self, command: Command) -> StepOutcome {
        let before = self.observable_snapshot();
        let current_before = self
            .current_identities()
            .into_iter()
            .collect::<HashSet<_>>();
        let operation_count = usize::from(command.0[0] % (MAX_TRANSACTION_OPERATIONS as u8 + 1));
        let array = pointer("/rows");
        let array_identity = self
            .identity_with_binding("/rows")
            .expect("the reviewed form has a rows array");
        let item = self.select_item(command.0[1]);
        let closure_failure = command.0[1] & 1 != 0;
        let movement = (operation_count == 1).then(|| match command.0[2] % 8 {
            6 => self.capture_movement(array_identity, item, MovementDirection::Up),
            7 => self.capture_movement(array_identity, item, MovementDirection::Down),
            _ => None,
        });
        let result = self.form.try_transact(|draft| {
            for index in 0..operation_count {
                match command.0[index + 2] % 8 {
                    0 => draft.set(&pointer("/name"), scalar_value(command.0[index + 3])),
                    1 => draft.remove(&pointer(if command.0[index + 3] & 1 == 0 {
                        "/optional"
                    } else {
                        "/missing"
                    })),
                    2 => draft.replace_all(root_value(command.0[index + 3])),
                    3 => draft.append_item(&array, row_value(command.0[index + 3])),
                    4 => draft.insert_item_before(&array, item, row_value(command.0[index + 3])),
                    5 => draft.remove_item(&array, item),
                    6 => draft.move_item_up(&array, item),
                    _ => draft.move_item_down(&array, item),
                }
            }
            if closure_failure { Err(()) } else { Ok(()) }
        });
        match result {
            Ok(transition) => {
                if let Some(Some(movement)) = movement {
                    self.assert_movement(&movement);
                }
                self.accepted(before, current_before, transition, "accepted")
            }
            Err(error) => {
                self.rejected(&before);
                let kind = match error {
                    TransactionError::Closure(()) => "closure-rejected",
                    TransactionError::Commit(HostCommitError::InvalidOperation) => {
                        "operation-rejected"
                    }
                    TransactionError::Commit(HostCommitError::ResourceLimit(_)) => "limit-rejected",
                    TransactionError::Commit(_) => "commit-rejected",
                    _ => "transaction-rejected",
                };
                StepOutcome::rejected(kind)
            }
        }
    }

    fn external_batch(&mut self, command: Command) -> StepOutcome {
        if command.0[0] % 4 == 3 {
            return self.external_data_change(command);
        }
        let before = self.observable_snapshot();
        let current_before = self
            .current_identities()
            .into_iter()
            .collect::<HashSet<_>>();
        let revision = match command.0[1] % 3 {
            0 => self.form.view().data_revision(),
            1 => self
                .stale_revision
                .unwrap_or_else(|| self.foreign_form.view().data_revision()),
            _ => self.foreign_form.view().data_revision(),
        };
        let encoded_count = command.0[2];
        let count = if encoded_count >= 247 {
            usize::from(encoded_count - 238)
        } else {
            usize::from(encoded_count % 9)
        };
        let expected_findings = (0..count)
            .map(|index| {
                let selector = command.0[3 + index % (COMMAND_BYTES - 3)];
                ExpectedExternalFinding::decode(selector)
            })
            .collect::<Vec<_>>();
        let mut findings = expected_findings
            .iter()
            .map(|finding| finding.external())
            .collect::<Vec<_>>();
        if command.0[0] & 1 != 0 {
            findings.reverse();
        }
        let source = if command.0[0] & 2 == 0 {
            ExternalSource::Server
        } else {
            ExternalSource::Policy
        };
        let mut expected_after = self.external_model.clone();
        let canonical = canonical_findings(expected_findings.clone());
        let expected_limit =
            external_batch_limit(&self.external_model, source, &expected_findings, &canonical);
        if canonical.is_empty() {
            expected_after.remove(&source);
        } else {
            expected_after.insert(source, canonical);
        }
        let current_revision = revision == before.data_revision;
        let expected_state_changed = expected_after != self.external_model;
        let result = self.form.apply_external_findings(ExternalFindingBatch::new(
            source.text(),
            revision,
            findings,
        ));
        match result {
            Ok(transition) => {
                assert!(
                    current_revision,
                    "a non-current external batch was accepted"
                );
                assert_eq!(
                    expected_limit, None,
                    "an over-limit external batch was accepted"
                );
                assert_eq!(
                    transition.after_data_revision(),
                    before.data_revision,
                    "external findings changed the data revision"
                );
                assert_eq!(
                    transition.before_state_revision() != transition.after_state_revision(),
                    expected_state_changed,
                    "external finding state/no-op behavior diverged from the model"
                );
                let outcome = self.accepted(before, current_before, transition, "accepted");
                self.external_model = expected_after;
                self.assert_external_model();
                outcome
            }
            Err(error) => {
                assert!(
                    !current_revision || expected_limit.is_some(),
                    "a model-accepted external batch was rejected: {error:?}"
                );
                if current_revision {
                    let expected_dimension = expected_limit.expect("current rejection has a limit");
                    assert!(matches!(
                        &error,
                        ExternalFindingError::ResourceLimit(limit)
                            if limit.dimension() == expected_dimension
                    ));
                } else {
                    assert!(matches!(&error, ExternalFindingError::StaleRevision { .. }));
                }
                self.rejected(&before);
                let kind = match error {
                    ExternalFindingError::StaleRevision { .. } => "revision-rejected",
                    ExternalFindingError::ResourceLimit(_) => "limit-rejected",
                    _ => "external-rejected",
                };
                StepOutcome::rejected(kind)
            }
        }
    }

    fn external_data_change(&mut self, command: Command) -> StepOutcome {
        let before = self.observable_snapshot();
        let current_before = self
            .current_identities()
            .into_iter()
            .collect::<HashSet<_>>();
        let value = format!("revision-{}", command.0[1] % 4);
        let transition = self
            .form
            .transact(|draft| draft.set(&pointer("/name"), json!(value)))
            .expect("bounded canonical external-model data change succeeds");
        let outcome = self.accepted(before, current_before, transition, "data-change");
        if outcome.data_changed {
            self.external_model.clear();
        }
        self.assert_external_model();
        outcome
    }

    fn accepted(
        &mut self,
        before: ObservableSnapshot,
        current_before: HashSet<InstanceIdentity>,
        transition: Transition,
        result: &'static str,
    ) -> StepOutcome {
        let after_data = self.form.view().data_revision();
        let after_state = self.form.view().state_revision();
        assert_eq!(transition.before_data_revision(), before.data_revision);
        assert_eq!(transition.before_state_revision(), before.state_revision);
        assert_eq!(transition.after_data_revision(), after_data);
        assert_eq!(transition.after_state_revision(), after_state);
        let data_changed = before.data_revision != after_data;
        let state_changed = before.state_revision != after_state;
        assert!(
            !data_changed || state_changed,
            "data changes must also advance state"
        );

        let changed_raw = transition.changed().collect::<Vec<_>>();
        let removed_raw = transition.removed().collect::<Vec<_>>();
        assert_unique(&changed_raw, "changed transition identities");
        assert_unique(&removed_raw, "removed transition identities");
        let current_after = self
            .current_identities()
            .into_iter()
            .collect::<HashSet<_>>();
        let expected_removed = current_before
            .difference(&current_after)
            .copied()
            .collect::<HashSet<_>>();
        assert_eq!(
            removed_raw.iter().copied().collect::<HashSet<_>>(),
            expected_removed,
            "transition removed identities are not the exact topology difference"
        );
        for identity in &changed_raw {
            assert!(
                current_after.contains(identity) && self.form.node(*identity).is_some(),
                "changed identity is not current"
            );
        }
        for identity in &removed_raw {
            assert!(
                current_before.contains(identity),
                "removed identity was not current"
            );
            assert!(
                self.form.node(*identity).is_none(),
                "removed identity remains addressable"
            );
        }

        if data_changed {
            self.stale_revision = Some(before.data_revision);
        }
        self.observe_identity_lifecycle(&current_before);
        for identity in &removed_raw {
            assert!(
                self.retired.contains(identity),
                "removed identity was not retired"
            );
        }
        let changed = changed_raw
            .into_iter()
            .map(|identity| self.identity_ordinal(identity))
            .collect();
        let removed = removed_raw
            .into_iter()
            .map(|identity| self.identity_ordinal(identity))
            .collect();
        StepOutcome {
            result,
            data_changed,
            state_changed,
            changed,
            removed,
        }
    }

    fn rejected(&mut self, before: &ObservableSnapshot) {
        assert_eq!(
            self.observable_snapshot(),
            *before,
            "rejection partially mutated form state"
        );
        let current = self
            .current_identities()
            .into_iter()
            .collect::<HashSet<_>>();
        self.observe_identity_lifecycle(&current);
    }

    fn observe_identity_lifecycle(&mut self, before: &HashSet<InstanceIdentity>) {
        let current = self.current_identities();
        let current_set = current.iter().copied().collect::<HashSet<_>>();
        assert_eq!(
            current.len(),
            current_set.len(),
            "current identities are not unique"
        );
        for identity in before.difference(&current_set) {
            self.retired.insert(*identity);
        }
        let current_items = self.current_item_roots();
        self.retired_items.extend(
            self.items
                .keys()
                .copied()
                .filter(|item| !current_items.contains(item)),
        );
        for identity in &current {
            assert!(
                !self.retired.contains(identity),
                "a retired identity was reused"
            );
            let was_known = self.identities.contains_key(identity);
            self.identity_ordinal(*identity);
            if !before.contains(identity) {
                assert!(
                    !was_known,
                    "a fresh identity aliases a previously observed identity"
                );
            }
            if let Some(item) = self
                .form
                .node(*identity)
                .and_then(|node| node.item_identity())
            {
                assert!(
                    !self.retired_items.contains(&item),
                    "a retired item identity was reused"
                );
                self.item_ordinal(item);
            }
        }
        for identity in &self.retired {
            assert!(
                self.form.node(*identity).is_none(),
                "retired identity is addressable"
            );
        }
    }

    fn capture_movement(
        &self,
        array: InstanceIdentity,
        item: ItemIdentity,
        direction: MovementDirection,
    ) -> Option<MovementExpectation> {
        let roots = self.item_roots(array)?;
        let index = roots.iter().position(|(_, candidate)| *candidate == item)?;
        let destination = match direction {
            MovementDirection::Up => index.saturating_sub(1),
            MovementDirection::Down => (index + 1).min(roots.len() - 1),
        };
        Some(MovementExpectation {
            array,
            item,
            destination,
            subtree: subtree_identities(&self.form, roots[index].0)
                .into_iter()
                .collect(),
        })
    }

    fn assert_movement(&self, expected: &MovementExpectation) {
        let roots = self
            .item_roots(expected.array)
            .expect("an accepted movement retained its array");
        let (root, item) = roots
            .get(expected.destination)
            .copied()
            .expect("an accepted movement retained its destination index");
        assert_eq!(
            item, expected.item,
            "movement targeted an array index instead of the selected logical item"
        );
        let subtree = subtree_identities(&self.form, root);
        assert_unique(&subtree, "moved item subtree identities");
        assert_eq!(
            subtree.into_iter().collect::<HashSet<_>>(),
            expected.subtree,
            "movement did not retain the complete selected item subtree"
        );
    }

    fn select_identity(&self, selector: u8, prefer_array: bool) -> InstanceIdentity {
        let current = self.current_identities();
        if selector % 5 == 3 {
            if let Some(identity) = self
                .retired
                .iter()
                .min_by_key(|identity| self.identities.get(identity).copied().unwrap_or(usize::MAX))
            {
                return *identity;
            }
        }
        if selector % 5 == 4 {
            return self.foreign_form.view().root();
        }
        let candidates = if prefer_array {
            current
                .iter()
                .copied()
                .filter(|identity| {
                    self.form.node(*identity).is_some_and(|node| {
                        node.definition().semantic_kind() == Some(SemanticKind::HomogeneousArray)
                    })
                })
                .collect::<Vec<_>>()
        } else {
            current.clone()
        };
        candidates
            .get(usize::from(selector) % candidates.len().max(1))
            .copied()
            .unwrap_or_else(|| current[usize::from(selector) % current.len()])
    }

    fn select_item(&self, selector: u8) -> ItemIdentity {
        if selector % 5 == 3 {
            if let Some(item) = self
                .retired_items
                .iter()
                .min_by_key(|item| self.items.get(item).copied().unwrap_or(usize::MAX))
            {
                return *item;
            }
        }
        let current = self.item_identities(&self.form);
        current
            .get(usize::from(selector) % current.len().max(1))
            .copied()
            .or_else(|| self.retired_items.iter().next().copied())
            .expect("the reviewed form has a current or retired item")
    }

    fn current_identities(&self) -> Vec<InstanceIdentity> {
        identities(&self.form)
    }

    fn item_identities(&self, form: &Form) -> Vec<ItemIdentity> {
        let mut seen = HashSet::new();
        identities(form)
            .into_iter()
            .filter_map(|identity| form.node(identity).and_then(|node| node.item_identity()))
            .filter(|item| seen.insert(*item))
            .collect()
    }

    fn current_item_roots(&self) -> HashSet<ItemIdentity> {
        let mut all = HashSet::new();
        for identity in self.current_identities() {
            if self.form.node(identity).is_some_and(|node| {
                node.definition().semantic_kind() == Some(SemanticKind::HomogeneousArray)
            }) {
                let roots = self
                    .item_roots(identity)
                    .expect("a current homogeneous array has current children");
                let items = roots.iter().map(|(_, item)| *item).collect::<Vec<_>>();
                assert_unique(&items, "immediate array item identities");
                all.extend(items);
            }
        }
        all
    }

    fn item_roots(&self, array: InstanceIdentity) -> Option<Vec<(InstanceIdentity, ItemIdentity)>> {
        let node = self.form.node(array)?;
        (node.definition().semantic_kind() == Some(SemanticKind::HomogeneousArray)).then(|| {
            node.children()
                .map(|identity| {
                    let item = self
                        .form
                        .node(identity)
                        .and_then(|child| child.item_identity())
                        .expect("an immediate repeated item root has an item identity");
                    (identity, item)
                })
                .collect()
        })
    }

    fn identity_with_binding(&self, binding: &str) -> Option<InstanceIdentity> {
        self.current_identities().into_iter().find(|identity| {
            self.form
                .node(*identity)
                .and_then(|node| node.binding())
                .is_some_and(|current| current.pointer().as_str() == binding)
        })
    }

    fn assert_external_model(&self) {
        let expected = self
            .external_model
            .iter()
            .flat_map(|(source, findings)| {
                findings.iter().map(move |finding| {
                    (
                        source.text().to_owned(),
                        finding.location.text().to_owned(),
                        finding.code.text().to_owned(),
                        finding.blocking,
                        finding.parameters.value(),
                    )
                })
            })
            .collect::<Vec<_>>();
        let actual = self
            .form
            .view()
            .visible_findings()
            .filter_map(|view| match view {
                FindingView::External {
                    source, finding, ..
                } => Some((
                    source.to_owned(),
                    finding.instance_location().as_str().to_owned(),
                    finding.code().to_owned(),
                    finding.is_blocking(),
                    finding.parameters().clone(),
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            actual, expected,
            "ordered visible external findings diverged from the model"
        );
    }

    fn identity_ordinal(&mut self, identity: InstanceIdentity) -> usize {
        if let Some(ordinal) = self.identities.get(&identity) {
            return *ordinal;
        }
        let ordinal = self.next_identity;
        self.next_identity += 1;
        self.identities.insert(identity, ordinal);
        ordinal
    }

    fn item_ordinal(&mut self, item: ItemIdentity) -> usize {
        if let Some(ordinal) = self.items.get(&item) {
            return *ordinal;
        }
        let ordinal = self.next_item;
        self.next_item += 1;
        self.items.insert(item, ordinal);
        ordinal
    }

    fn observable_snapshot(&mut self) -> ObservableSnapshot {
        ObservableSnapshot {
            data: self.form.form_data().clone(),
            data_revision: self.form.view().data_revision(),
            state_revision: self.form.view().state_revision(),
            normalized: self.normalized_snapshot(),
        }
    }

    fn normalized_snapshot(&mut self) -> NormalizedSnapshot {
        for identity in identities(&self.form) {
            self.identity_ordinal(identity);
            if let Some(item) = self
                .form
                .node(identity)
                .and_then(|node| node.item_identity())
            {
                self.item_ordinal(item);
            }
        }
        let validation = match self.form.view().validation_outcome() {
            ValidationOutcomeView::Valid => "valid".to_owned(),
            ValidationOutcomeView::Invalid {
                findings,
                truncated,
            } => format!(
                "invalid:{truncated}:{}",
                findings
                    .iter()
                    .map(|finding| format!(
                        "{}|{}|{}|{}",
                        finding.instance_location().as_str(),
                        finding.keyword_location().pointer().as_str(),
                        finding.code(),
                        finding.parameters()
                    ))
                    .collect::<Vec<_>>()
                    .join(";")
            ),
            ValidationOutcomeView::Indeterminate(reason) => {
                format!("indeterminate:{}", reason.code())
            }
        };
        let view = self.form.view();
        let raw_findings = view.visible_findings().collect::<Vec<_>>();
        let findings = raw_findings
            .into_iter()
            .map(|finding| self.normalize_finding(finding))
            .collect();
        let raw_nodes = identities(&self.form)
            .into_iter()
            .map(|identity| {
                let node = self
                    .form
                    .node(identity)
                    .expect("traversed identity remains current");
                (
                    identity,
                    node.item_identity(),
                    format!("{:?}", node.definition().kind()),
                    format!("{:?}", node.definition().semantic_kind()),
                    node.binding()
                        .map(|binding| binding.pointer().as_str().to_owned()),
                    format!("{:?}", node.allowed_operations()),
                    node.edit_buffer().map(str::to_owned),
                    node.parse_blocker().map(|kind| format!("{kind:?}")),
                    node.is_touched(),
                    node.is_dirty(),
                    node.current_data().cloned(),
                    node.display_text(),
                )
            })
            .collect::<Vec<_>>();
        let nodes = raw_nodes
            .into_iter()
            .map(|node| NormalizedNode {
                identity: self.identity_ordinal(node.0),
                item: node.1.map(|item| self.item_ordinal(item)),
                kind: node.2,
                semantic_kind: node.3,
                binding: node.4,
                allowed: node.5,
                edit_buffer: node.6,
                parse_blocker: node.7,
                touched: node.8,
                dirty: node.9,
                current_data: node.10,
                display_text: node.11,
            })
            .collect();
        NormalizedSnapshot {
            data: self.form.form_data().clone(),
            validation,
            submission_attempted: self.form.view().submission_attempted(),
            findings,
            nodes,
        }
    }

    fn normalize_finding(&self, finding: FindingView<'_>) -> String {
        match finding {
            FindingView::Validation { target, finding } => format!(
                "validation|{}|{}|{}|{}",
                self.known_identity_ordinal(target),
                finding.instance_location().as_str(),
                finding.code(),
                finding.parameters()
            ),
            FindingView::ValidationFindingsTruncated { target, retained } => {
                format!(
                    "validation-truncated|{}|{retained}",
                    self.known_identity_ordinal(target)
                )
            }
            FindingView::Indeterminate { target, reason } => format!(
                "indeterminate|{}|{}",
                self.known_identity_ordinal(target),
                reason.code()
            ),
            FindingView::Capability { target, finding } => format!(
                "capability|{}|{}|{}",
                self.known_identity_ordinal(target),
                finding.code(),
                finding.parameters()
            ),
            FindingView::External {
                target,
                source,
                finding,
            } => format!(
                "external|{}|{source}|{}|{}|{}|{}",
                self.known_identity_ordinal(target),
                finding.instance_location().as_str(),
                finding.code(),
                finding.is_blocking(),
                finding.parameters()
            ),
            FindingView::Parse { target, kind } => {
                format!("parse|{}|{kind:?}", self.known_identity_ordinal(target))
            }
            _ => "unknown-finding-family".to_owned(),
        }
    }

    fn known_identity_ordinal(&self, identity: InstanceIdentity) -> usize {
        *self
            .identities
            .get(&identity)
            .expect("observable finding targets are current identities")
    }
}

impl StepOutcome {
    fn rejected(result: &'static str) -> Self {
        Self {
            result,
            data_changed: false,
            state_changed: false,
            changed: Vec::new(),
            removed: Vec::new(),
        }
    }
}

impl ExpectedExternalFinding {
    const fn decode(selector: u8) -> Self {
        Self {
            location: ExternalLocation::decode(selector),
            code: ExternalCode::decode(selector),
            blocking: selector & 1 == 0,
            parameters: ExternalParameters::decode(selector),
        }
    }

    fn external(self) -> ExternalFinding {
        if self.blocking {
            ExternalFinding::blocking(
                self.code.text(),
                pointer(self.location.text()),
                self.parameters.value(),
            )
        } else {
            ExternalFinding::advisory(
                self.code.text(),
                pointer(self.location.text()),
                self.parameters.value(),
            )
        }
    }

    const fn encoded_bytes(self) -> usize {
        self.location.encoded_bytes() + self.code.encoded_bytes() + self.parameters.encoded_bytes()
    }
}

impl ExternalSource {
    const fn text(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::Server => "server",
        }
    }

    const fn encoded_bytes(self) -> usize {
        match self {
            Self::Policy | Self::Server => 6,
        }
    }

    const fn ordering_rank(self) -> u8 {
        match self {
            Self::Policy => 0,
            Self::Server => 1,
        }
    }
}

impl Ord for ExternalSource {
    fn cmp(&self, other: &Self) -> Ordering {
        self.ordering_rank().cmp(&other.ordering_rank())
    }
}

impl PartialOrd for ExternalSource {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl ExternalLocation {
    const fn decode(selector: u8) -> Self {
        match selector % 4 {
            0 => Self::Name,
            1 => Self::Count,
            2 => Self::RowValue,
            _ => Self::LongUnmatched,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Self::Name => "/name",
            Self::Count => "/count",
            Self::RowValue => "/rows/0/value",
            Self::LongUnmatched => "/unmatched/extended-external-finding-location",
        }
    }

    const fn ordering_rank(self) -> u8 {
        match self {
            Self::Count => 0,
            Self::Name => 1,
            Self::RowValue => 2,
            Self::LongUnmatched => 3,
        }
    }

    const fn encoded_bytes(self) -> usize {
        match self {
            Self::Name => 5,
            Self::Count => 6,
            Self::RowValue => 13,
            Self::LongUnmatched => 45,
        }
    }
}

impl ExternalCode {
    const fn decode(selector: u8) -> Self {
        match selector % 3 {
            0 => Self::Finding0,
            1 => Self::Finding1,
            _ => Self::Finding2,
        }
    }

    const fn text(self) -> &'static str {
        match self {
            Self::Finding0 => "finding-0",
            Self::Finding1 => "finding-1",
            Self::Finding2 => "finding-2",
        }
    }

    const fn ordering_rank(self) -> u8 {
        match self {
            Self::Finding0 => 0,
            Self::Finding1 => 1,
            Self::Finding2 => 2,
        }
    }

    const fn encoded_bytes(self) -> usize {
        match self {
            Self::Finding0 | Self::Finding1 | Self::Finding2 => 9,
        }
    }
}

impl ExternalParameters {
    const fn decode(selector: u8) -> Self {
        if selector & 1 == 0 {
            Self::Zero
        } else {
            Self::One
        }
    }

    fn value(self) -> Value {
        match self {
            Self::Zero => json!({ "v": 0 }),
            Self::One => json!({ "v": 1 }),
        }
    }

    const fn ordering_rank(self) -> u8 {
        match self {
            Self::Zero => 0,
            Self::One => 1,
        }
    }

    const fn encoded_bytes(self) -> usize {
        match self {
            Self::Zero | Self::One => 7,
        }
    }
}

fn canonical_findings(mut findings: Vec<ExpectedExternalFinding>) -> Vec<ExpectedExternalFinding> {
    findings.sort_by_key(|finding| {
        (
            finding.location.ordering_rank(),
            finding.code.ordering_rank(),
            u8::from(finding.blocking),
            finding.parameters.ordering_rank(),
        )
    });
    findings.dedup();
    findings
}

fn external_batch_limit(
    batches: &BTreeMap<ExternalSource, Vec<ExpectedExternalFinding>>,
    replacement_source: ExternalSource,
    incoming_findings: &[ExpectedExternalFinding],
    replacement_findings: &[ExpectedExternalFinding],
) -> Option<&'static str> {
    if incoming_findings.len() > MAX_INCOMING_EXTERNAL_FINDINGS {
        return Some("incoming_external_findings");
    }
    if incoming_external_batch_bytes(replacement_source, incoming_findings)
        > MAX_INCOMING_EXTERNAL_FINDING_BYTES
    {
        return Some("incoming_external_finding_bytes");
    }
    let retained_count = batches
        .iter()
        .filter(|(source, _)| **source != replacement_source)
        .map(|(_, findings)| findings.len())
        .sum::<usize>();
    let count = retained_count.saturating_add(replacement_findings.len());
    if count > MAX_ACTIVE_EXTERNAL_FINDINGS {
        return Some("active_external_findings");
    }
    let retained_bytes = batches
        .iter()
        .filter(|(source, _)| **source != replacement_source)
        .map(|(source, findings)| external_batch_bytes(*source, findings))
        .sum::<usize>();
    let bytes = retained_bytes.saturating_add(external_batch_bytes(
        replacement_source,
        replacement_findings,
    ));
    (bytes > MAX_ACTIVE_EXTERNAL_FINDING_BYTES).then_some("active_external_finding_bytes")
}

#[cfg(test)]
fn external_model_bytes(batches: &BTreeMap<ExternalSource, Vec<ExpectedExternalFinding>>) -> usize {
    batches
        .iter()
        .map(|(source, findings)| external_batch_bytes(*source, findings))
        .sum()
}

fn external_batch_bytes(source: ExternalSource, findings: &[ExpectedExternalFinding]) -> usize {
    if findings.is_empty() {
        return 0;
    }
    findings
        .iter()
        .fold(source.encoded_bytes(), |total, finding| {
            total.saturating_add(finding.encoded_bytes())
        })
}

fn incoming_external_batch_bytes(
    source: ExternalSource,
    findings: &[ExpectedExternalFinding],
) -> usize {
    findings
        .iter()
        .fold(source.encoded_bytes(), |total, finding| {
            total.saturating_add(finding.encoded_bytes())
        })
}

fn build_form() -> Form {
    let duplicate = row_value(0);
    FormDefinition::compile(reviewed_runtime_schema())
        .expect("reviewed runtime schema compiles")
        .form(json!({
            "name": "Ada",
            "count": 1,
            "enabled": true,
            "optional": "present",
            "rows": [duplicate.clone(), duplicate]
        }))
        .limits(
            FormDataLimits::default()
                .max_depth(8)
                .max_nodes(64)
                .max_members(64)
                .max_collection_length(6)
                .max_scalar_bytes(16)
                .max_form_tree_nodes(64)
                .max_repeated_items(4)
                .max_edit_buffer_bytes(8)
                .max_active_edit_buffers(4)
                .max_total_edit_buffer_bytes(16)
                .max_host_operations_per_transaction(4)
                .max_retained_validation_findings(8)
                .max_validation_parameter_bytes(64),
        )
        .finding_visibility(FindingVisibilityPolicy::new(
            FindingVisibility::Immediate,
            FindingVisibility::Immediate,
        ))
        .external_finding_limits(ExternalFindingLimits::new(
            NonZeroUsize::new(MAX_ACTIVE_EXTERNAL_FINDINGS).expect("nonzero limit"),
            NonZeroUsize::new(MAX_ACTIVE_EXTERNAL_FINDING_BYTES).expect("nonzero limit"),
        ))
        .build()
        .expect("reviewed runtime form builds")
}

fn reviewed_runtime_schema() -> Value {
    json!({
        "$schema": DIALECT,
        "type": "object",
        "additionalProperties": false,
        "required": ["name", "count", "enabled", "rows"],
        "properties": {
            "name": { "type": ["string", "null"] },
            "count": { "type": "integer" },
            "enabled": { "type": "boolean" },
            "optional": { "type": "string" },
            "profile": {
                "type": "object",
                "default": { "note": "seed" },
                "additionalProperties": false,
                "properties": { "note": { "type": "string" } }
            },
            "rows": {
                "type": "array",
                "maxItems": 4,
                "items": {
                    "type": "object",
                    "default": { "value": "same", "score": 1 },
                    "additionalProperties": false,
                    "required": ["value", "score"],
                    "properties": {
                        "value": { "type": "string" },
                        "score": { "type": "integer" }
                    }
                }
            }
        }
    })
}

fn identities(form: &Form) -> Vec<InstanceIdentity> {
    let mut pending = vec![form.view().root()];
    let mut result = Vec::new();
    while let Some(identity) = pending.pop() {
        let node = form
            .node(identity)
            .expect("traversed identity remains current");
        let mut children = node.children().collect::<Vec<_>>();
        children.reverse();
        pending.extend(children);
        result.push(identity);
    }
    result
}

fn subtree_identities(form: &Form, root: InstanceIdentity) -> Vec<InstanceIdentity> {
    let mut pending = vec![root];
    let mut result = Vec::new();
    while let Some(identity) = pending.pop() {
        let node = form
            .node(identity)
            .expect("a captured item subtree identity remains current");
        pending.extend(node.children());
        result.push(identity);
    }
    result
}

fn assert_unique<T: Eq + std::hash::Hash + Copy>(values: &[T], description: &str) {
    assert_eq!(
        values.len(),
        values.iter().copied().collect::<HashSet<_>>().len(),
        "{description} are not unique"
    );
}

fn pointer(value: &str) -> JsonPointer {
    JsonPointer::parse(value).expect("reviewed pointer is valid")
}

fn text_value(selector: u8) -> &'static str {
    ["", "x", "same", "-", "123", "too-long-value"][usize::from(selector % 6)]
}

fn scalar_value(selector: u8) -> Value {
    match selector % 5 {
        0 => json!("same"),
        1 => json!(selector),
        2 => json!(selector & 1 == 0),
        3 => Value::Null,
        _ => json!({ "unexpected": true }),
    }
}

fn replacement_value(selector: u8) -> Value {
    match selector % 3 {
        0 => json!("replacement"),
        1 => row_value(selector),
        _ => json!([]),
    }
}

fn row_value(selector: u8) -> Value {
    json!({ "value": if selector & 1 == 0 { "same" } else { "other" }, "score": selector % 4 })
}

fn root_value(selector: u8) -> Value {
    match selector % 3 {
        0 => {
            json!({ "name": "Host", "count": 2, "enabled": false, "rows": [row_value(0), row_value(0)] })
        }
        1 => json!([]),
        _ => json!({ "name": "Host", "rows": [] }),
    }
}

fn user_error_kind(error: &UserOperationError) -> &'static str {
    match error {
        UserOperationError::UnknownTarget => "unknown-target",
        UserOperationError::OperationNotAllowed => "operation-not-allowed",
        UserOperationError::ResourceLimit(_) => "limit-rejected",
        _ => "user-rejected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_items_become_reachable_stale_selectors() {
        let mut runtime = Runtime::new(Program::User);
        let array = runtime.identity_with_binding("/rows").unwrap();
        let item = runtime.item_roots(array).unwrap()[0].1;
        let before = runtime.observable_snapshot();
        let current_before = runtime.current_identities().into_iter().collect();
        let transition = runtime.form.user().remove_item(array, item).unwrap();

        runtime.accepted(before, current_before, transition, "accepted");

        assert!(runtime.retired_items.contains(&item));
        assert_eq!(runtime.select_item(3), item);
    }

    #[test]
    fn duplicate_valued_movement_retains_the_selected_subtree() {
        let mut runtime = Runtime::new(Program::User);
        let array = runtime.identity_with_binding("/rows").unwrap();
        let roots = runtime.item_roots(array).unwrap();
        assert_eq!(
            runtime.form.node(roots[0].0).unwrap().current_data(),
            runtime.form.node(roots[1].0).unwrap().current_data()
        );
        let expected = runtime
            .capture_movement(array, roots[1].1, MovementDirection::Up)
            .unwrap();

        runtime.form.user().move_item_up(array, roots[1].1).unwrap();

        runtime.assert_movement(&expected);
    }

    #[test]
    fn canonical_data_changes_clear_external_model_and_stale_old_revisions() {
        let mut runtime = Runtime::new(Program::External);
        runtime.external_batch(Command([0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(
            runtime.external_model.values().map(Vec::len).sum::<usize>(),
            1
        );

        runtime.external_batch(Command([3, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert!(runtime.external_model.is_empty());

        let stale = runtime.external_batch(Command([0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(stale.result, "revision-rejected");
        assert!(runtime.external_model.is_empty());
    }

    #[test]
    fn duplicate_external_findings_are_canonicalized_before_count_limits() {
        let mut runtime = Runtime::new(Program::External);

        let outcome = runtime.external_batch(Command([0, 0, 5, 0, 0, 0, 0, 0, 0, 0, 0, 0]));

        assert_eq!(outcome.result, "accepted");
        assert_eq!(runtime.external_model[&ExternalSource::Server].len(), 1);
    }

    #[test]
    fn empty_external_replacements_contribute_zero_active_bytes() {
        assert_eq!(external_batch_bytes(ExternalSource::Server, &[]), 0);
        assert_eq!(
            incoming_external_batch_bytes(ExternalSource::Server, &[]),
            ExternalSource::Server.encoded_bytes()
        );
    }

    #[test]
    fn external_model_checks_raw_limits_before_active_limits() {
        let duplicate = ExpectedExternalFinding::decode(0);
        let over_count = vec![duplicate; MAX_INCOMING_EXTERNAL_FINDINGS + 1];
        let canonical = canonical_findings(over_count.clone());
        assert_eq!(
            external_batch_limit(
                &BTreeMap::new(),
                ExternalSource::Server,
                &over_count,
                &canonical,
            ),
            Some("incoming_external_findings")
        );

        let long = ExpectedExternalFinding::decode(3);
        let over_bytes = vec![long; 9];
        let canonical = canonical_findings(over_bytes.clone());
        assert!(
            external_batch_bytes(ExternalSource::Server, &over_bytes)
                > MAX_INCOMING_EXTERNAL_FINDING_BYTES
        );
        assert_eq!(
            external_batch_limit(
                &BTreeMap::new(),
                ExternalSource::Server,
                &over_bytes,
                &canonical,
            ),
            Some("incoming_external_finding_bytes")
        );
    }

    #[test]
    fn decoded_external_batches_reach_raw_limits_atomically() {
        let mut count_runtime = Runtime::new(Program::External);
        let over_count =
            count_runtime.external_batch(Command([0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
        assert_eq!(over_count.result, "limit-rejected");
        assert!(count_runtime.external_model.is_empty());

        let mut byte_runtime = Runtime::new(Program::External);
        let over_bytes =
            byte_runtime.external_batch(Command([0, 0, 247, 3, 3, 3, 3, 3, 3, 3, 3, 3]));
        assert_eq!(over_bytes.result, "limit-rejected");
        assert!(byte_runtime.external_model.is_empty());
    }

    #[test]
    fn external_byte_limit_is_reachable_below_count_limit_and_is_atomic() {
        let byte_findings = canonical_findings(
            [3, 7, 11]
                .into_iter()
                .map(ExpectedExternalFinding::decode)
                .collect(),
        );
        let byte_model = BTreeMap::from([(ExternalSource::Server, byte_findings)]);
        assert_eq!(byte_model.values().map(Vec::len).sum::<usize>(), 3);
        assert_eq!(external_model_bytes(&byte_model), 6 + 3 * (45 + 9 + 7));
        assert_eq!(external_model_bytes(&byte_model), 189);
        assert_eq!(
            external_batch_limit(
                &BTreeMap::new(),
                ExternalSource::Server,
                &byte_model[&ExternalSource::Server],
                &byte_model[&ExternalSource::Server]
            ),
            Some("active_external_finding_bytes")
        );

        let mut byte_runtime = Runtime::new(Program::External);
        let byte_before = byte_runtime.observable_snapshot();
        let byte_rejected =
            byte_runtime.external_batch(Command([0, 0, 3, 3, 7, 11, 0, 0, 0, 0, 0, 0]));
        assert_eq!(byte_rejected.result, "limit-rejected");
        assert_eq!(byte_runtime.observable_snapshot(), byte_before);
        assert!(byte_runtime.external_model.is_empty());
    }

    #[test]
    fn external_count_limit_is_reachable_below_byte_limit_and_is_atomic() {
        let count_findings = canonical_findings(
            [0, 1, 2, 4, 5]
                .into_iter()
                .map(ExpectedExternalFinding::decode)
                .collect(),
        );
        let count_model = BTreeMap::from([(ExternalSource::Server, count_findings)]);
        assert_eq!(count_model.values().map(Vec::len).sum::<usize>(), 5);
        assert_eq!(external_model_bytes(&count_model), 121);
        assert_eq!(
            external_batch_limit(
                &BTreeMap::new(),
                ExternalSource::Server,
                &count_model[&ExternalSource::Server],
                &count_model[&ExternalSource::Server]
            ),
            Some("active_external_findings")
        );

        let mut count_runtime = Runtime::new(Program::External);
        let count_before = count_runtime.observable_snapshot();
        let count_rejected =
            count_runtime.external_batch(Command([0, 0, 5, 0, 1, 2, 4, 5, 0, 0, 0, 0]));
        assert_eq!(count_rejected.result, "limit-rejected");
        assert_eq!(count_runtime.observable_snapshot(), count_before);
        assert!(count_runtime.external_model.is_empty());
    }
}
