# Business schema corpus

This corpus is executable product-path evidence for the schemaform capability profile. It contains attributed, realistic Draft 2020-12 data-schema excerpts that cover onboarding, billing, account settings, product configuration, addresses, line items, and nested preferences. Corpus qualification proves the declared outcome of each adapted fixture; it is not a compatibility claim for every upstream schema.

The fixtures are intentionally offline. Source URLs identify provenance only; corpus tests load only the resources declared in `manifest.json`.

## Fixture contract

Each manifest entry records:

- Upstream publisher, immutable source revision, path, dialect, vocabulary set, license, retrieval date, and adaptation notes.
- One root schema and any caller-supplied supporting resources, each with an offline retrieval URI.
- Every exercised schema keyword, linked by stable profile ID to the authoritative machine-readable cases in `testing/support-profile.json`; `docs/support-profile.md` explains the profile for humans. A construct entry classifies every occurrence in that fixture; recorded locations provide representative evidence, and the test rejects mixed contexts that would require separate profile cases.
- Lexical schema depth, declared property count, homogeneous or tuple collection shapes, locally optional properties, nullable patterns, composition, and every reference target.
- Representative in-profile semantic controls and layouts, whether generated UI is sufficient, first-release authored UI needs, known deferred UI needs, and expected capability findings.

Schema locations use `<resource-name>#<JSON Pointer>`. They identify locations in a data schema, not control bindings into form data.

`max_schema_depth` starts at zero for each resource root and increases when traversing a schema-valued keyword without following references. `property_count` counts names under every `properties` object, including properties inside applicator branches. Optionality is local to the nearest `properties` and `required` pair; it does not claim resolved composition semantics.

## Scope

The integrity check validates attribution, manifest closure, schema shape, keyword classification, and agreement with the capability profile. Every resource is validated against the Draft 2020-12 meta-schema, and every reference graph is checked offline.

The independent product check embeds the same manifest and all 23 resources, then drives every fixture through `FormCompiler`. All 20 fixtures qualify. The 15 in-profile fixtures compile strictly, create a form, and prepare submission without a capability blocker. The five out-of-profile fixtures return the manifest's exact finding codes, severity, resource identities, and keyword pointers from strict and lenient compilation; their lenient forms preserve canonical data and block submission.

The real-browser test mounts every in-profile fixture through `use_form`, `RenderConfiguration::default().bind`, and `SchemaForm`, then submits each rendered form. The interaction manifest requires that trace in Chromium, Firefox, and WebKit matrix evidence. No fixture installs a custom renderer.

Controls and layouts remain representative critical needs rather than compatibility promises for complete upstream schemas. [ADR 0004](../../../docs/adr/0004-validate-trusted-schemas-with-stock-jsonschema.md) governs the first-release trusted-schema validator; [ADR 0002](../../../docs/adr/0002-bound-validation-work.md) remains future hostile-schema work.

This directory is data-schema product evidence, not the UI-schema compatibility
corpus. Authored presentation needs use stable UI schema v1. Its accepted and
rejected wire fixtures live under
`crates/schemaform/tests/fixtures/ui-schema-v1`, and its canonical
meta-schema is `ui-schema-v1.schema.json`. Adding an authored UI document
for a business fixture would prove that fixture's presentation path, not imply
compatibility with an upstream project's UI format.

Run the integrity and core product checks with:

```sh
cargo test --locked -p schemaform \
  --test business_schema_corpus --test business_schema_product
```

The browser command in the repository `README.md` runs the adapter trace with the rest of `browser_csr`.

## Licensing

Fixture schemas are form-oriented adapted excerpts, not verbatim mirrors. Reductions preserve the upstream business concepts, while each manifest entry records structural changes such as inlining resources, selecting required fields, closing object excerpts, or normalizing field names. Each changed file carries a `$comment` notice and records the upstream license and immutable source. Apache-2.0 fixture material is distributed under the repository's `LICENSE-APACHE`; the hubverse excerpt is sourced under CC0-1.0. Attribution does not imply upstream endorsement.
