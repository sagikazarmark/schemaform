# Schemaform

[![OpenSSF Scorecard](https://api.securityscorecards.dev/projects/github.com/sagikazarmark/schemaform/badge?style=flat-square)](https://securityscorecards.dev/viewer/?uri=github.com/sagikazarmark/schemaform)
[![crates.io](https://img.shields.io/crates/v/schemaform?style=flat-square)](https://crates.io/crates/schemaform)
[![docs.rs](https://img.shields.io/docsrs/schemaform?style=flat-square)](https://docs.rs/schemaform)

**Runtime JSON Schema forms for [Dioxus](https://dioxuslabs.com): a
framework-neutral form engine plus an accessible browser adapter.**

Schemaform builds forms whose structure is discovered at runtime from
application-trusted JSON Schema Draft 2020-12 documents. The repository contains
two packages: the Dioxus-free `schemaform` core and the browser-CSR
`schemaform-dioxus` adapter.

| Package | What it does |
| --- | --- |
| [`schemaform`](crates/schemaform/README.md) | Compiles a data schema into a reusable definition, owns canonical form data and edit state, validates every accepted change, and prepares immutable submission snapshots. No Dioxus dependency. |
| [`schemaform-dioxus`](crates/schemaform-dioxus/README.md) | Renders a definition as accessible, unstyled semantic HTML in a Dioxus browser client-side-rendered application, with renderer, finding presenter, localization, and extension seams. |

## Features

- **Runtime compilation:** Draft 2020-12 data schemas compile into an immutable
  definition tree, including local references, caller-supplied in-memory
  resource graphs, canonical `$id` values, relative cross-resource references,
  and named anchors. Resource retrieval is denied.
- **Scalar controls:** strings, arbitrary-precision numbers and integers,
  booleans, finite choices, nullable scalars, null-only values, and constants.
  Missing, null, empty, compatible, and incompatible states are preserved and
  repaired through explicit operations rather than render-time mutation.
- **Fixed objects:** nested objects, optional objects that materialize only
  through explicit actions, open objects that preserve and validate undeclared
  members, and compatible `allOf` branches that contribute controls,
  requiredness, constraints, and provenance without merging schema documents.
- **Homogeneous arrays:** opaque per-item identity, including for duplicate
  values, with identity-targeted append, insert-before, remove, and move
  operations. Row state, findings, focus, and DOM identity follow the logical
  item when indices shift.
- **Validation and findings:** structured findings carry stable keyword codes,
  instance and schema locations, and code-specific parameters. Host-supplied
  external findings are replaced by source at an exact data revision under
  finite count and byte limits. Submission returns either every structured
  blocker or one immutable data/revision/fingerprint snapshot.
- **Annotations:** `title` and `description` become accessible label and help
  text; `readOnly` removes user mutation authority including for descendants;
  `writeOnly` keeps canonical values out of the DOM while offering explicit
  replacement. Defaults apply only when a user explicitly creates or repairs a
  value.
- **Optional UI schema:** stable version 1 of the complete vocabulary — all
  seven elements, explicit generation, inline item templates, element IDs,
  localizable text and item labels, exact widgets, finding presenters, and
  exact-URI extensions.
- **Explicit capability boundaries:** unsupported editing semantics produce
  located, typed capability findings instead of being guessed or silently
  omitted. Strict compilation blocks; lenient analysis renders the region
  without weakening validation or submission gates.

## Status

Both packages are published on crates.io and share one version; see
[CHANGELOG.md](CHANGELOG.md) for what changed in each release. Browser latency
and runtime-memory calibration remain future work, and no release makes a
quantitative claim for either.

## Quick Start

Compile a trusted data schema, edit through the core engine, and prepare a
submission snapshot:

```rust
use schemaform::{FormDefinition, SubmissionOutcome};
use serde_json::json;

let definition = FormDefinition::compile(json!({
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "additionalProperties": false,
    "required": ["name"],
    "properties": {
        "name": { "type": "string", "title": "Name", "minLength": 1 }
    }
}))?;

let mut form = definition.create_form(json!({ "name": "Ada" }))?;
let name = form
    .node(form.view().root())
    .and_then(|root| root.children().next())
    .expect("the generated name control should exist");

form.user().input_text(name, "Grace")?;

let (_transition, outcome) = form.prepare_submission().into_parts();
match outcome {
    SubmissionOutcome::Ready(snapshot) => {
        assert_eq!(snapshot.form_data(), &json!({ "name": "Grace" }));
    }
    SubmissionOutcome::Blocked(blockers) => {
        eprintln!("blocked by {} finding(s)", blockers.iter().count());
    }
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

Render the same definition in a Dioxus browser application:

```rust
use dioxus::prelude::*;
use schemaform::FormDefinition;
use schemaform_dioxus::{RenderConfiguration, SchemaForm, use_form};
use serde_json::json;

#[component]
fn App() -> Element {
    let definition = use_hook(|| FormDefinition::compile(json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["name"],
        "properties": {
            "name": { "type": "string", "title": "Your name" }
        }
    })).expect("the trusted data schema should compile"));
    let form = use_form(definition, json!({ "name": "Ada" }))
        .expect("the form should be created");
    let form_to_bind = form.clone();
    let bound = use_hook(move || RenderConfiguration::default()
        .bind(&form_to_bind)
        .expect("the built-in renderer should bind"));

    rsx! {
        SchemaForm {
            form: bound,
            on_submit: move |snapshot: schemaform::SubmissionSnapshot| {
                println!("{}", snapshot.form_data());
            },
            on_error: move |error| eprintln!("form operation failed: {error}"),
        }
    }
}
```

See each package README for compiler options, error tables, custom renderers,
and localization.

## Capability Boundary

The first release supports a non-null fixed-object root containing supported
scalars, nested fixed objects, and one homogeneous array per root-to-leaf
branch. Fixed-object array items may contain nested fixed objects but not
arrays.

Nullable support applies only to scalar controls: nullable fixed objects and
nullable arrays are capability-blocking. Optional container properties may be
absent but do not accept null. Schema-valued additional properties and pattern
properties warn when the declared projection is fixed, and block when they are
required to determine the editable members.

Homogeneous array composition and collection actions stay adapter-owned, so the
Dioxus adapter rejects array-level widget requests during render binding while
still preflighting inline item templates. Eligible item controls may use custom
renderers. This adapter boundary does not change the stable UI-schema v1 wire
format or headless compilation semantics.

Data schemas must be application-trusted. The core meta-validates schemas,
denies implicit I/O, and applies finite structural limits, but does not contain
hostile evaluator workloads.

## UI Schema

The optional UI-schema path implements stable version 1 of the complete
vocabulary. The [v1 meta-schema](ui-schema-v1.schema.json) and its
accepted/rejected fixtures freeze accepted JSON documents and their
framework-neutral headless meaning independently of crate SemVer.

That promise does not freeze DOM or component structure, HTML shape or
identity, IDs, classes, data attributes, CSS, styling, breakpoints, focus or
ARIA implementation details, or future accessibility corrections.

## Demo

The docs-by-example Dioxus application in [`demo/`](demo/README.md) covers
generated controls, authored UI schemas, homogeneous arrays, and a live schema
playground:

```console
cd demo
npm ci
npm run build
dx serve
```

## Development

Run the native workspace checks with:

```console
cargo test --locked --workspace
```

The browser tracer uses `wasm-bindgen-test-runner` matching the locked
`wasm-bindgen` version, Firefox, and geckodriver:

```console
RUSTFLAGS="--cfg schemaform_test_validation_faults" \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER="$(command -v wasm-bindgen-test-runner)" \
GECKODRIVER="$(command -v geckodriver)" \
cargo test --locked --target wasm32-unknown-unknown \
  -p schemaform-dioxus --test browser_csr
```

The same public-facade corpora run natively and in browser WASM, so the
fixed-object, array, and business-schema qualification suites can be executed
against either target:

```console
cargo test --locked -p schemaform \
  --test fixed_object_profile_target --test scalar_array_facade \
  --test fixed_object_array_facade --test deferred_shape_projection_target \
  --test business_schema_corpus --test business_schema_product
```

All 20 attributed business-schema fixtures run through the production core
compiler. The 15 in-profile fixtures also create forms, prepare submission, and
mount through the default Dioxus browser adapter without custom renderers; the
five out-of-profile fixtures have exact strict and lenient typed capability
outcomes.

Verify the checked-in browser contracts and replay retained fuzz inputs with:

```console
cargo run --locked -p browser-workload-pack -- check
cargo test --locked -p schemaform-fuzz-harness
```

### Release gates

Publication resolves one existing tag to an exact commit, and reusable native,
fuzz, and browser workflows independently check out and verify that commit. It
is gated on native workspace tests, both `SCHEMAFORM_PROPTEST_PROFILE=release`
model tests, all seven two-hour-per-target release fuzz budgets plus
retained-corpus replay, and the complete browser gate. Qualification logs, fuzz
corpora and findings, replay logs, and browser evidence are retained for 90
days.

The interaction gate runs the browser-neutral real-DOM suite in every pinned
Chromium, Firefox, and WebKit cell at 320 and 1280 CSS pixels and 100 and 200
percent zoom, injecting pinned `axe-core` at manifest-declared DOM checkpoints.
Start the interactive test server:

```console
RUSTFLAGS="--cfg schemaform_test_validation_faults" \
NO_HEADLESS=1 \
CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER="$(command -v wasm-bindgen-test-runner)" \
cargo test --locked --target wasm32-unknown-unknown \
  -p schemaform-dioxus --test browser_csr
```

Then run and verify the matrix with the pinned Playwright image and package:

```console
docker run --rm --network host \
  -e WASM_BINDGEN_TEST_URL=http://127.0.0.1:8000 \
  -v "$PWD:/work" -w /work \
  mcr.microsoft.com/playwright:v1.55.0-noble@sha256:b27e719ecbfef153e13fd24e8341736733bf2658b229677eb21ff57ff5d7fb29 \
  sh -c 'npm ci --ignore-scripts && node testing/browser/scripts/run-browser-interaction-matrix.js'
cargo run --locked -p browser-workload-pack -- \
  verify-interactions testing/browser/artifacts/interaction-observation.json
```

The production-artifact size gate is defined by the hashed
`testing/browser/workload-pack/artifact-manifest.json` and enforces fixed caps
of 1536 KiB total Brotli WASM, 512 KiB incremental Brotli WASM over the empty
shell, and 64 KiB total Brotli runtime JavaScript. These byte counts require no
browser, performance hardware, baseline, or calibrated ceiling:

```console
cargo run --locked -p browser-workload-pack -- verify-artifacts
```

After the interaction matrix and artifact-size gate pass, archive and
independently verify the complete browser evidence set. The archive rejects
missing browser traces and accessibility reports, artifacts, manifest sidecars,
passing conclusions, retries, waivers, dirty source, changed bytes, and
unreferenced objects:

```console
cargo run --locked -p browser-workload-pack -- \
  archive-evidence testing/browser/artifacts/evidence
cargo run --locked -p browser-workload-pack -- \
  verify-evidence testing/browser/artifacts/evidence
```

See the [testing layout](testing/README.md) for how the shared fixtures,
browser packs, and fuzz harness fit together.

## Domain Language

The public form terms used by this project are defined in
[`CONTEXT.md`](CONTEXT.md). The glossary describes the domain contract without
documenting private module or backend layout.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
