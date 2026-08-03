#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod browser {
    use std::{cell::RefCell, sync::Arc};

    use browser_workload_pack::{Scenario, SetupOperation, Workload};
    use dioxus::prelude::*;
    use schemaform::{
        CompilationProfile, ExternalFinding, ExternalFindingBatch, FindingVisibility,
        FindingVisibilityPolicy, FormDefinition, JsonPointer, json::parse_ui_schema_v1,
    };
    use schemaform_dioxus::{
        FormHandle, Localizer, RenderConfiguration, SchemaForm,
        render::{BoundForm, MessageDescriptor},
        use_form,
    };
    use serde_json::json;
    use wasm_bindgen::prelude::*;

    thread_local! {
        static ACTIVE: RefCell<Option<ActiveScenario>> = const { RefCell::new(None) };
        static PREPARED: RefCell<Option<(Scenario, FormDefinition)>> = const { RefCell::new(None) };
    }

    #[derive(Clone)]
    struct ActiveScenario {
        scenario: Scenario,
        form: FormHandle,
        bound: BoundForm,
        commit_token: Signal<u32>,
    }

    #[wasm_bindgen]
    pub struct WorkloadInvocation {
        started_at_ms: f64,
        commit_token: u32,
    }

    #[wasm_bindgen]
    impl WorkloadInvocation {
        #[wasm_bindgen(getter)]
        pub fn started_at_ms(&self) -> f64 {
            self.started_at_ms
        }

        #[wasm_bindgen(getter)]
        pub fn commit_token(&self) -> u32 {
            self.commit_token
        }
    }

    #[wasm_bindgen]
    pub struct ColdInvocation {
        started_at_ms: f64,
        finished_at_ms: f64,
    }

    #[wasm_bindgen]
    impl ColdInvocation {
        #[wasm_bindgen(getter)]
        pub fn started_at_ms(&self) -> f64 {
            self.started_at_ms
        }

        #[wasm_bindgen(getter)]
        pub fn finished_at_ms(&self) -> f64 {
            self.finished_at_ms
        }
    }

    #[wasm_bindgen]
    pub fn compile_workload(serialized_scenario: &str) -> Result<ColdInvocation, JsValue> {
        let started_at_ms = now()?;
        compile_scenario(serialized_scenario)
            .map(|_| ())
            .map_err(|error| JsValue::from_str(&error))?;
        Ok(ColdInvocation {
            started_at_ms,
            finished_at_ms: now()?,
        })
    }

    #[wasm_bindgen]
    pub fn prepare_mount_workload(serialized_scenario: &str) -> Result<(), JsValue> {
        if ACTIVE.with_borrow(Option::is_some) || PREPARED.with_borrow(Option::is_some) {
            return Err(JsValue::from_str("workload runner is already prepared"));
        }
        let prepared =
            compile_scenario(serialized_scenario).map_err(|error| JsValue::from_str(&error))?;
        PREPARED.with_borrow_mut(|slot| *slot = Some(prepared));
        Ok(())
    }

    #[wasm_bindgen]
    pub fn mount_workload() -> Result<f64, JsValue> {
        if ACTIVE.with_borrow(Option::is_some) {
            return Err(JsValue::from_str("workload runner is already mounted"));
        }
        if PREPARED.with_borrow(Option::is_none) {
            return Err(JsValue::from_str("workload runner is not prepared"));
        }
        let started_at_ms = now()?;
        dioxus_web::launch::launch_cfg(app, Default::default());
        Ok(started_at_ms)
    }

    fn app() -> Element {
        let (mut scenario, definition) = PREPARED
            .with_borrow_mut(Option::take)
            .expect("workload runner must be prepared before mount");
        let id = scenario.id.clone();
        let form = use_form(definition, scenario.initial_form_data.clone())
            .expect("workload form must construct");
        let setup = std::mem::take(&mut scenario.setup);
        let setup_form = form.clone();
        use_hook(move || apply_setup(&setup_form, setup));
        let bound = RenderConfiguration::default()
            .bind(&form)
            .expect("workload form must bind");
        let commit_token = use_signal(|| 0_u32);
        ACTIVE.with_borrow_mut(|active| {
            *active = Some(ActiveScenario {
                scenario,
                form: form.clone(),
                bound: bound.clone(),
                commit_token,
            });
        });

        rsx! {
            main {
                "data-workload-scenario": id,
                SchemaForm {
                    form: bound,
                    on_submit: move |_| {},
                    on_error: move |_| {},
                }
                CommitSentinel { token: commit_token, form: form.clone() }
            }
        }
    }

    #[component]
    fn CommitSentinel(mut token: Signal<u32>, form: FormHandle) -> Element {
        let token = token();
        let state_revision = format!(
            "{:?}",
            form.reader()
                .read()
                .expect("the workload form should not be mutably borrowed while rendering")
                .state_revision
        );
        rsx! {
            output {
                id: "workload-commit-sentinel",
                "aria-hidden": "true",
                "data-workload-commit": token,
                "data-workload-state-revision": state_revision,
            }
        }
    }

    #[wasm_bindgen]
    pub fn run_workload(name: &str, phase: usize) -> Result<WorkloadInvocation, JsValue> {
        let started_at_ms = now()?;
        let active = ACTIVE
            .with_borrow(Clone::clone)
            .ok_or_else(|| JsValue::from_str("workload runner is not mounted"))?;
        let workload = active
            .scenario
            .workloads
            .iter()
            .find(|workload| workload.name() == name)
            .ok_or_else(|| JsValue::from_str("scenario does not define that workload"))?;
        execute_workload(&active, workload, phase).map_err(|error| JsValue::from_str(&error))?;
        let mut token = active.commit_token;
        let commit_token = {
            let mut value = token.write();
            *value += 1;
            *value
        };
        Ok(WorkloadInvocation {
            started_at_ms,
            commit_token,
        })
    }

    fn compile_scenario(serialized_scenario: &str) -> Result<(Scenario, FormDefinition), String> {
        let scenario: Scenario = serde_json::from_str(serialized_scenario)
            .map_err(|error| format!("parse workload scenario: {error}"))?;
        let profile = CompilationProfile::standard();
        let mut compiler = FormDefinition::compiler(scenario.data_schema.clone());
        if let Some(ui_schema) = &scenario.ui_schema {
            let bytes = serde_json::to_vec(ui_schema)
                .map_err(|error| format!("serialize UI schema: {error}"))?;
            let ui_schema = parse_ui_schema_v1(&bytes, &profile)
                .map_err(|error| format!("parse UI schema: {error:?}"))?;
            compiler = compiler.ui_schema(ui_schema);
        }
        let definition = compiler
            .profile(profile)
            .compile()
            .map_err(|error| format!("compile workload definition: {error:?}"))?;
        Ok((scenario, definition))
    }

    fn now() -> Result<f64, JsValue> {
        web_sys::window()
            .and_then(|window| window.performance())
            .map(|performance| performance.now())
            .ok_or_else(|| JsValue::from_str("browser performance clock is unavailable"))
    }

    fn apply_setup(form: &FormHandle, setup: Vec<SetupOperation>) {
        for operation in setup {
            match operation {
                SetupOperation::InputText { binding, value } => {
                    node_for_binding(form, &binding)
                        .actions()
                        .input_text(value)
                        .expect("parse-blocked setup must apply");
                }
                SetupOperation::ExternalFindings { count, binding } => {
                    apply_findings(form, &binding, count, "browser-workload-findings")
                        .expect("finding setup must apply");
                }
            }
        }
    }

    fn execute_workload(
        active: &ActiveScenario,
        workload: &Workload,
        phase: usize,
    ) -> Result<(), String> {
        match workload {
            Workload::Compilation { .. } | Workload::Mount { .. } => Ok(()),
            Workload::Edit {
                binding,
                alternating_values,
            } => node_for_binding(&active.form, binding)
                .actions()
                .input_text(alternating_values[phase % 2].clone())
                .map(|_| ())
                .map_err(|error| error.to_string()),
            Workload::Findings {
                binding,
                count,
                alternating_actions,
            } => {
                let count = if alternating_actions[phase % 2] == "install" {
                    *count
                } else {
                    0
                };
                apply_findings(&active.form, binding, count, "browser-workload-findings")
            }
            Workload::Visibility { policies } => {
                let visibility = match policies[phase % 2].as_str() {
                    "immediate" => FindingVisibility::Immediate,
                    "submission-only" => FindingVisibility::SubmissionOnly,
                    policy => return Err(format!("unknown finding visibility policy: {policy}")),
                };
                active
                    .form
                    .set_finding_visibility(FindingVisibilityPolicy::new(visibility, visibility))
                    .map(|_| ())
                    .map_err(|error| error.to_string())
            }
            Workload::Arrays {
                binding,
                operations,
            } => {
                let node = node_for_binding(&active.form, binding);
                let actions = node.collection_actions();
                match operations[phase % operations.len()].as_str() {
                    "append" => actions.append().map(|_| ()),
                    "remove-last" => actions.remove(last_item(&node)).map(|_| ()),
                    "insert-before-last" => actions.insert_before(last_item(&node)).map(|_| ()),
                    "remove-before-last" => actions.remove(before_last_item(&node)).map(|_| ()),
                    "move-last-up" => actions.move_up(last_item(&node)).map(|_| ()),
                    "move-before-last-down" => {
                        actions.move_down(before_last_item(&node)).map(|_| ())
                    }
                    operation => return Err(format!("unknown array operation: {operation}")),
                }
                .map_err(|error| error.to_string())
            }
            Workload::Localization { locales, .. } => {
                RenderConfiguration::builder()
                    .localizer(Arc::new(WorkloadLocalizer {
                        locale: locales[phase % 2].clone(),
                    }))
                    .build()
                    .rebind_presentation(&active.bound);
                Ok(())
            }
            Workload::Submission { .. } => active
                .form
                .prepare_submission()
                .map(|_| ())
                .map_err(|error| error.to_string()),
        }
    }

    fn apply_findings(
        form: &FormHandle,
        binding: &str,
        count: usize,
        source: &str,
    ) -> Result<(), String> {
        let revision = form
            .reader()
            .read()
            .map_err(|error| error.to_string())?
            .data_revision;
        let pointer = JsonPointer::parse(binding).map_err(|error| error.to_string())?;
        let findings = (0..count)
            .map(|index| {
                ExternalFinding::blocking(
                    format!("workload-{index:02}"),
                    pointer.clone(),
                    json!({ "index": index }),
                )
            })
            .collect::<Vec<_>>();
        form.apply_external_findings(ExternalFindingBatch::new(source, revision, findings))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn node_for_binding(form: &FormHandle, binding: &str) -> schemaform_dioxus::NodeReader {
        let mut pending = vec![
            form.reader()
                .read()
                .expect("the workload form should not be mutably borrowed while resolving nodes")
                .root,
        ];
        while let Some(identity) = pending.pop() {
            let node = form
                .node(identity)
                .expect("the workload form should not be mutably borrowed while resolving nodes")
                .expect("form projection identity must exist");
            let projection = node
                .read()
                .expect("the workload form should not be mutably borrowed while resolving nodes")
                .expect("form projection identity must remain readable");
            if projection
                .binding
                .as_ref()
                .is_some_and(|candidate| candidate.as_str() == binding)
            {
                return node;
            }
            pending.extend(projection.children);
        }
        panic!("scenario has no node for binding {binding}")
    }

    fn last_item(node: &schemaform_dioxus::NodeReader) -> schemaform::ItemIdentity {
        node.read()
            .expect("the workload form should not be mutably borrowed while resolving items")
            .and_then(|projection| projection.collection_items.last().copied())
            .map(|item| item.item)
            .expect("array workload requires at least one item")
    }

    fn before_last_item(node: &schemaform_dioxus::NodeReader) -> schemaform::ItemIdentity {
        let projection = node
            .read()
            .expect("the workload form should not be mutably borrowed while resolving items")
            .expect("array workload node must remain readable");
        projection.collection_items[projection.collection_items.len() - 2].item
    }

    struct WorkloadLocalizer {
        locale: String,
    }

    impl Localizer for WorkloadLocalizer {
        fn localize(&self, message: &MessageDescriptor) -> String {
            format!("{}:{}", self.locale, message.fallback)
        }
    }
}
