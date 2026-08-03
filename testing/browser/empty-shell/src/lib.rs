#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod browser {
    use std::cell::RefCell;

    use browser_workload_pack::Scenario;
    use dioxus::prelude::*;
    use wasm_bindgen::prelude::*;

    thread_local! {
        static PREPARED: RefCell<Option<Scenario>> = const { RefCell::new(None) };
        static TOKEN: RefCell<Option<Signal<u32>>> = const { RefCell::new(None) };
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
        parse_scenario(serialized_scenario)?;
        Ok(ColdInvocation {
            started_at_ms,
            finished_at_ms: now()?,
        })
    }

    #[wasm_bindgen]
    pub fn prepare_mount_workload(serialized_scenario: &str) -> Result<(), JsValue> {
        if PREPARED.with_borrow(Option::is_some) {
            return Err(JsValue::from_str("empty shell is already prepared"));
        }
        let scenario = parse_scenario(serialized_scenario)?;
        PREPARED.with_borrow_mut(|slot| *slot = Some(scenario));
        Ok(())
    }

    #[wasm_bindgen]
    pub fn mount_workload() -> Result<f64, JsValue> {
        if PREPARED.with_borrow(Option::is_none) {
            return Err(JsValue::from_str("empty shell is not prepared"));
        }
        let started_at_ms = now()?;
        dioxus_web::launch::launch_cfg(app, Default::default());
        Ok(started_at_ms)
    }

    #[wasm_bindgen]
    pub fn run_workload(_name: &str, _phase: usize) -> Result<WorkloadInvocation, JsValue> {
        let started_at_ms = now()?;
        let mut token = TOKEN
            .with_borrow(|token| *token)
            .ok_or_else(|| JsValue::from_str("empty shell is not mounted"))?;
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

    fn app() -> Element {
        let scenario = PREPARED
            .with_borrow_mut(Option::take)
            .expect("empty shell must be prepared before mount");
        let token = use_signal(|| 0_u32);
        TOKEN.with_borrow_mut(|slot| *slot = Some(token));
        let token_value = token();
        rsx! {
            main {
                "data-workload-scenario": scenario.id,
                "Schemaform qualification shell"
                output {
                    id: "workload-commit-sentinel",
                    "aria-hidden": "true",
                    "data-workload-commit": token_value,
                }
            }
        }
    }

    fn parse_scenario(serialized_scenario: &str) -> Result<Scenario, JsValue> {
        serde_json::from_str(serialized_scenario)
            .map_err(|error| JsValue::from_str(&format!("parse workload scenario: {error}")))
    }

    fn now() -> Result<f64, JsValue> {
        web_sys::window()
            .and_then(|window| window.performance())
            .map(|performance| performance.now())
            .ok_or_else(|| JsValue::from_str("browser performance clock is unavailable"))
    }
}
