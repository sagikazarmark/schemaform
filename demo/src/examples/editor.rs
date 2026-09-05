use dioxus::prelude::*;
use dioxus_code_editor::{CodeEditor, Language};
use schemaform::{
    CompilationProfile, FormDefinition,
    json::{parse_data_schema, parse_ui_schema_v1},
};
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

use crate::components::{
    DemoPane, DemoSurface, StatusChip, StatusLine, schemaform_daisyui, snippet_theme,
};

const DATA_SCHEMA: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "type": "object",
  "additionalProperties": false,
  "required": ["name", "age", "newsletter", "role"],
  "properties": {
    "name": {
      "type": "string",
      "title": "Display name",
      "minLength": 3
    },
    "age": {
      "type": "integer",
      "title": "Age",
      "minimum": 18
    },
    "newsletter": {
      "type": "boolean",
      "title": "Send product updates"
    },
    "role": {
      "title": "Workspace role",
      "enum": ["admin", "editor", "viewer"]
    }
  }
}"#;

const UI_SCHEMA: &str = r#"{
  "version": 1,
  "root": {
    "type": "stack",
    "value": {
      "children": [
        {
          "type": "text",
          "value": {
            "content": {
              "fallback": "Edit either schema, then apply it to this preview."
            }
          }
        },
        {
          "type": "grid",
          "value": {
            "cells": [
              {
                "compact_span": 12,
                "wide_span": 8,
                "child": {
                  "type": "control",
                  "value": {
                    "binding": { "origin": "root", "pointer": "/name" }
                  }
                }
              },
              {
                "compact_span": 12,
                "wide_span": 4,
                "child": {
                  "type": "control",
                  "value": {
                    "binding": { "origin": "root", "pointer": "/age" }
                  }
                }
              }
            ]
          }
        },
        {
          "type": "tabs",
          "value": {
            "panels": [
              {
                "title": { "fallback": "Access" },
                "child": {
                  "type": "control",
                  "value": {
                    "binding": { "origin": "root", "pointer": "/role" }
                  }
                }
              },
              {
                "title": { "fallback": "Notifications" },
                "child": {
                  "type": "control",
                  "value": {
                    "binding": { "origin": "root", "pointer": "/newsletter" }
                  }
                }
              }
            ]
          }
        }
      ]
    }
  }
}"#;

#[derive(Clone)]
struct CompiledDefinition(FormDefinition);

impl PartialEq for CompiledDefinition {
    fn eq(&self, other: &Self) -> bool {
        self.0.fingerprint() == other.0.fingerprint()
    }
}

#[derive(Clone, Copy, PartialEq)]
enum SchemaTab {
    Data,
    Ui,
}

/// A two-tab JSON editor drives a compiled form preview. Invalid JSON or an
/// unsupported schema reports an error and leaves the last valid preview
/// mounted, so authoring mistakes never destroy the working form.
#[component]
pub fn SchemaEditorExample() -> Element {
    let mut data_schema = use_signal(|| DATA_SCHEMA.to_owned());
    let mut ui_schema = use_signal(|| UI_SCHEMA.to_owned());
    let mut tab = use_signal(|| SchemaTab::Data);
    let mut definition = use_signal(|| {
        CompiledDefinition(
            compile_sources(DATA_SCHEMA, UI_SCHEMA)
                .expect("the initial playground schemas should compile"),
        )
    });
    let mut revision = use_signal(|| 0_u64);
    let mut result = use_signal(|| Ok::<_, String>("Schemas are valid.".to_owned()));

    rsx! {
        DemoSurface {
            primary: rsx! {
                DemoPane {
                    label: "Schemas",
                    accessory: rsx! { StatusChip { label: "Draft 2020-12 + UI v1" } },
                    div { class: "schema-editor overflow-hidden rounded-xl border border-base-300 bg-base-100",
                        div {
                            role: "group",
                            "aria-label": "Schema editors",
                            class: "tabs tabs-border px-3 pt-2",
                            button {
                                r#type: "button",
                                class: if tab() == SchemaTab::Data { "tab tab-active" } else { "tab" },
                                "aria-pressed": tab() == SchemaTab::Data,
                                onclick: move |_| tab.set(SchemaTab::Data),
                                "JSON schema"
                            }
                            button {
                                r#type: "button",
                                class: if tab() == SchemaTab::Ui { "tab tab-active" } else { "tab" },
                                "aria-pressed": tab() == SchemaTab::Ui,
                                onclick: move |_| tab.set(SchemaTab::Ui),
                                "UI schema"
                            }
                        }
                        if tab() == SchemaTab::Data {
                            CodeEditor {
                                class: "max-h-[34rem] overflow-auto font-mono text-sm",
                                value: data_schema.read().clone(),
                                language: Language::Json,
                                theme: snippet_theme(),
                                spellcheck: false,
                                aria_label: "JSON data schema editor",
                                oninput: move |value: String| data_schema.set(value),
                            }
                        } else {
                            CodeEditor {
                                class: "max-h-[34rem] overflow-auto font-mono text-sm",
                                value: ui_schema.read().clone(),
                                language: Language::Json,
                                theme: snippet_theme(),
                                spellcheck: false,
                                aria_label: "UI schema editor",
                                oninput: move |value: String| ui_schema.set(value),
                            }
                        }
                    }
                    div { class: "mt-3 flex flex-wrap items-center gap-3",
                        button {
                            class: "btn btn-sm btn-primary",
                            r#type: "button",
                            onclick: move |_| {
                                match compile_sources(&data_schema.peek(), &ui_schema.peek()) {
                                    Ok(compiled) => {
                                        definition.set(CompiledDefinition(compiled));
                                        revision += 1;
                                        result.set(Ok("Schemas applied to the preview.".to_owned()));
                                    }
                                    Err(error) => result.set(Err(error)),
                                }
                            },
                            "Apply schemas"
                        }
                        match result.read().as_ref() {
                            Ok(message) => rsx! {
                                span { role: "status", class: "text-sm text-success", "{message}" }
                            },
                            Err(error) => rsx! {
                                span { role: "alert", class: "text-sm text-error", "{error} Preview unchanged." }
                            },
                        }
                    }
                }
            },
            secondary: rsx! {
                DemoPane {
                    label: "Rendered form",
                    accessory: rsx! { StatusChip { label: "revision {revision}" } },
                    div {
                        key: "{revision}",
                        class: "rounded-xl border border-base-300 bg-base-100 p-4",
                        PlaygroundForm { definition: definition.read().clone() }
                    }
                }
            },
        }
    }
}

/// The form the playground renders for its current definition: built-in controls and layouts,
/// with the shell, any arrays, and the finding summary from the demo's daisyUI component.
#[component]
fn PlaygroundForm(definition: CompiledDefinition) -> Element {
    let form = use_form(definition.0, initial_form_data());
    let Ok(form) = form else {
        return rsx! {
            p { role: "alert", class: "text-sm text-error", "The initial form data is outside this form's structural limits." }
        };
    };
    let bound_form = form.clone();
    let bound = use_hook(move || {
        RenderConfiguration::builder()
            .structure(schemaform_daisyui::structure())
            .summary_presenter(schemaform_daisyui::findings())
            .build()
            .bind(&bound_form)
            .expect("playground compilation preflights default renderer requirements")
    });
    let mut submitted = use_signal(String::new);

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                submitted.set(
                    serde_json::to_string_pretty(snapshot.form_data())
                        .expect("form data should serialize"),
                );
            },
            on_error: move |error| crate::examples::report_form_error(&error),
        }
        StatusLine { status: submitted.read().clone() }
    }
}

fn compile_sources(data_schema: &str, ui_schema: &str) -> Result<FormDefinition, String> {
    let profile = CompilationProfile::default();
    let data_schema = parse_data_schema(data_schema.as_bytes(), &profile)
        .map_err(|error| format!("Data schema: {error}"))?;
    let ui_schema = parse_ui_schema_v1(ui_schema.as_bytes(), &profile)
        .map_err(|error| format!("UI schema: {error}"))?;
    let definition = FormDefinition::compiler(data_schema)
        .ui_schema(ui_schema)
        .compile()
        .map_err(|error| format!("Compilation: {error}"))?;

    if definition.required_extensions().next().is_some() {
        return Err("Rendering: the default preview has no extension handlers.".to_owned());
    }
    let mut pending = vec![definition.root()];
    while let Some(id) = pending.pop() {
        let node = definition
            .node(id)
            .expect("definition traversal only contains valid node IDs");
        if node.widget().is_some() {
            return Err(
                "Rendering: the default preview has no custom widget renderers.".to_owned(),
            );
        }
        pending.extend(node.children());
    }
    definition
        .create_form(initial_form_data())
        .map_err(|error| format!("Form data: {error}"))?;

    Ok(definition)
}

fn initial_form_data() -> serde_json::Value {
    json!({
        "name": "Ada",
        "age": 36,
        "newsletter": true,
        "role": "admin"
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn initial_playground_schemas_compile() {
        super::compile_sources(super::DATA_SCHEMA, super::UI_SCHEMA).unwrap();
    }
}
