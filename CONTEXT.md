# Schemaform

This context describes forms whose structure and data shape are discovered at runtime rather than fixed by Rust types.

## Language

**Data schema**:
A JSON Schema document that describes the accepted shape, constraints, and annotations of form data.
_Avoid_: Schema, form schema

**Trusted data schema**:
A data schema whose provenance the host application accepts for evaluation without a deterministic evaluation-work bound. The library may still reject malformed inputs and implicit resource retrieval, but does not claim that trusted-schema mode safely contains hostile evaluator workloads.
_Avoid_: Safe schema, bounded schema

**UI schema**:
An optional, versioned JSON document that describes presentation and interaction concerns not expressed by the data schema. It may borrow ideas from Eclipse JSON Forms, but compatibility with that project's UI schema is not a goal.
_Avoid_: JSON Forms UI schema, layout schema

**Item template**:
An inline UI-schema subtree owned by a homogeneous array control and instantiated once per array item. Its control bindings are template-origin; runtime array-item identity is not part of the template.
_Avoid_: Row schema, reusable template

**Definition tree**:
The immutable, framework-neutral hierarchy of layouts, text, controls, item templates, and explicitly unsupported regions produced by compiling a data schema and UI schema, before form data or runtime array-item identity is applied.
_Avoid_: Compiled form tree, render tree, component tree

**Form tree**:
The runtime instantiation of a definition tree, including each node's current semantic state and repeated item instances.
_Avoid_: Definition tree, render tree, component tree, view tree

**Instance identity**:
A library-owned opaque identity for one instantiated node in a form tree. It remains attached to the same logical repeated item as that item moves, while its control binding and array index may change.
_Avoid_: JSON Pointer, array index, node key

**Control binding**:
An RFC 6901 JSON Pointer that identifies the location in form data edited by a control. Bindings are root-origin unless they are relative to the current array item within an item template.
_Avoid_: Scope, field path, schema path

**Fixed object**:
An object shape whose editable projection is a finite set of statically named properties. Undeclared members may still be preserved and validated; supporting them does not imply dynamic-key editing.
_Avoid_: Closed object, static object

**Node presentation**:
Adapter-computed, localized presentation data for one form-tree node: its suggested element id, label, help, local findings with stable ids, presence affordances, and invalid state. A renderer that receives it owns the elements it references; the adapter renders nothing on the renderer's behalf.
_Avoid_: Accessibility data, field meta, control context

**Presence operation**:
A user operation that changes whether a value exists at a control binding — materialize, set, set null, remove, replace — rather than editing its content. The core decides which presence operations a node allows right now.
_Avoid_: Nullability toggle, clear

**Affordance**:
A localized, pre-authorized user action handed to a renderer. Invoking it performs the core operation and reports failures to the host; collection affordances also announce and move focus. Renderers place affordances, they do not compose them.
_Avoid_: Button, action callback

**Form data**:
The canonical JSON instance currently being edited and validated by a form.
_Avoid_: Model, form value, values

**Baseline**:
The canonical form data and repeated-item identity topology that reset restores and dirty state compares against. Explicit reinitialization replaces the baseline; ordinary edits and host transactions do not.
_Avoid_: Initial data, defaults, original value

**Data revision**:
A form-scoped opaque revision that advances once when an atomic operation changes canonical form data. Explicit reinitialization advances it even when the new form data is semantically equal.
_Avoid_: Data version, state revision

**State revision**:
A form-scoped opaque revision that advances once when an atomic operation makes any observable form-state change, including form data, interaction metadata, edit buffers, policy, or findings.
_Avoid_: Data revision, render version

**Validation outcome**:
The result of evaluating form data against its data schema: valid, invalid, or indeterminate.
_Avoid_: Validation status, validation result

**Validation finding**:
A structured explanation of an invalid validation outcome, identified by instance and data-schema locations with keyword-specific parameters.
_Avoid_: Validation message, validator error

**External finding**:
A revision-scoped finding supplied by the host about current form data. It has an instance location but does not require a data-schema location.
_Avoid_: Validation finding, server error

**Finding visibility**:
Whether a current finding should be presented under the form's configured policy and interaction state. Visibility never changes whether the finding blocks submission.
_Avoid_: Finding validity, blocking state

**Indeterminate validation**:
A validation outcome where validity could not be determined because evaluation could not complete reliably, including exhaustion of an applicable evaluator resource limit. It prevents submission rather than being treated as valid or invalid.
_Avoid_: Validation timeout, validation error

**Evaluation-work budget**:
A finite allowance for data-schema evaluation effort. It is distinct from wall-clock timeouts and static input-size limits; exhausting it produces indeterminate validation.
_Avoid_: Validation timeout, schema size limit

**Submission snapshot**:
An immutable copy of a submittable form-data revision returned after the core finalizes edit buffers and checks all blockers.
_Avoid_: Payload, submitted form, request body

**Edit buffer**:
The exact temporary textual value of an active text-like edit. It may preserve a parseable spelling after form data has changed or retain unparseable input that cannot yet change form data.
_Avoid_: Raw value, invalid value

**Dirty state**:
Whether current canonical form data differs semantically from its baseline at a form-tree node. Interaction metadata and edit-buffer spelling do not make form data dirty, and JSON numbers compare by mathematical value.
_Avoid_: Changed state, touched state, modified flag

**Parse blocker**:
A finding that prevents an edit buffer from becoming form data and therefore prevents the current form from being submitted.
_Avoid_: Validation error, parse error

**Capability finding**:
A finding that a valid data schema or UI schema construct cannot be represented faithfully in the form tree.
_Avoid_: Validation error, unsupported-schema error
