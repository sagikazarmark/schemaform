use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    error::Error,
    fmt,
};

use jsonptr::PointerBuf;
use num_bigint::BigInt;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{
    CompilationLimitDimension, CompilationLimitError, CompilationLimitPhase, CompilationProfile,
    RetrievalUri,
    resources::{GraphLocation, ResourceGraph},
};

const DEFAULT_MAX_CANONICAL_INTEGER_DIGITS: usize = 4096;

#[derive(Clone)]
pub struct FormDefinition {
    controls: Vec<ControlDefinition>,
    objects: Vec<ObjectDefinition>,
    arrays: Vec<ArrayDefinition>,
    unsupported_regions: Vec<UnsupportedRegion>,
    capability_warnings: Vec<(PointerBuf, UnsupportedFinding)>,
    root_schema_locations: Vec<SchemaLocationDefinition>,
    root_annotations: DataSchemaAnnotations,
    fingerprint: DefinitionFingerprint,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct DefinitionFingerprint([u8; 32]);

impl fmt::Debug for DefinitionFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DefinitionFingerprint(..)")
    }
}

impl FormDefinition {
    pub fn compile(data_schema: Value) -> Result<Self, CompileError> {
        Self::compile_at(data_schema, "urn:schemaform:root")
    }

    pub fn compile_at(data_schema: Value, root_resource: &str) -> Result<Self, CompileError> {
        let root_uri = RetrievalUri::parse(root_resource.to_owned())
            .map_err(|_| CompileError::UnsupportedReference(root_resource.to_owned()))?;
        let graph = ResourceGraph::prepare(root_uri, data_schema, Vec::new())
            .map_err(|_| CompileError::UnsupportedReference(root_resource.to_owned()))?;
        Self::compile_graph(
            &graph,
            CompilationProfile::default().data_schema_limits().traversal,
        )
    }

    pub(crate) fn compile_graph(
        graph: &ResourceGraph,
        max_traversal: usize,
    ) -> Result<Self, CompileError> {
        let mut used_resources = BTreeSet::from([graph.root_resource().to_owned()]);
        let mut traversal = ProjectionTraversal::new(max_traversal);
        let mut validation_traversal = ProjectionTraversal::new(max_traversal);
        include_validation_resources(
            graph,
            graph.root_location(),
            &mut used_resources,
            &mut validation_traversal,
        )?;
        let root_expansion = expand_references(
            graph,
            vec![located_from_graph(graph.root_location())],
            &BTreeSet::new(),
            &mut used_resources,
            &mut traversal,
        )?;
        let root_applicable = root_expansion.schemas;
        let root_schema_locations = schema_locations(&root_applicable);
        let root_annotations = data_schema_annotations(&root_applicable);
        let (root_title, root_title_warning) = text_annotation(&root_applicable, "title");
        let (_, root_description_warning) = text_annotation(&root_applicable, "description");
        let root_kind = infer_kind(&root_applicable);
        let (root_has_scalar_choices, root_choice_findings) =
            match scalar_choices(&root_applicable, root_kind) {
                Ok(choices) => (choices.is_some(), Vec::new()),
                Err(findings) => (false, findings),
            };
        let mut capability_warnings = Vec::new();
        for warning in [root_description_warning, root_title_warning]
            .into_iter()
            .flatten()
        {
            capability_warnings.push((PointerBuf::new(), warning));
        }
        if root_kind == Some(ProjectedKind::Object) {
            capability_warnings.extend(open_object_warnings(&root_applicable, PointerBuf::new()));
        }
        let mut root_findings = root_expansion.recursive_findings;
        root_findings.extend(deferred_shape_findings(
            graph,
            &root_applicable,
            root_kind,
            root_has_scalar_choices,
        ));
        root_findings.extend(root_choice_findings.iter().cloned());
        root_findings.extend(
            all_of_kind_conflicts(&root_applicable)
                .into_iter()
                .map(|origin| all_of_unsupported_finding(origin, "incompatible-kind")),
        );
        if root_kind == Some(ProjectedKind::Object) {
            root_findings.extend(dynamic_object_map_findings(&root_applicable));
        } else if root_choice_findings.is_empty() {
            root_findings.push(unsupported_root_finding(
                &root_applicable,
                root_kind,
                root_has_scalar_choices,
            ));
        }
        let mut unsupported_regions = Vec::new();
        if !root_findings.is_empty() {
            unsupported_regions.push(unsupported_region(
                PointerBuf::new(),
                None,
                root_schema_locations.clone(),
                NodePresentation {
                    label: root_title.unwrap_or_else(|| "Form".to_owned()),
                    help: None,
                    annotations: root_annotations.clone(),
                },
                true,
                root_findings,
            ));
        }
        if root_kind != Some(ProjectedKind::Object) {
            debug_assert!(!unsupported_regions.is_empty());
            let controls = Vec::new();
            let fingerprint =
                fingerprint_compiled_definition(&controls, &[], &[], graph, &used_resources);
            return Ok(Self {
                controls,
                objects: Vec::new(),
                arrays: Vec::new(),
                unsupported_regions,
                capability_warnings,
                root_schema_locations,
                root_annotations,
                fingerprint,
            });
        }
        let mut controls = Vec::new();
        let mut objects = Vec::new();
        let mut arrays = Vec::new();
        if root_applicable
            .iter()
            .any(|located| located.schema.get("properties").is_some())
        {
            let active_locations = root_applicable.iter().map(schema_identity).collect();
            let mut projection = ProjectionState {
                controls: &mut controls,
                objects: &mut objects,
                arrays: &mut arrays,
                unsupported_regions: &mut unsupported_regions,
                capability_warnings: &mut capability_warnings,
                used_resources: &mut used_resources,
                traversal: &mut traversal,
                inside_array_template: false,
            };
            compile_properties(
                graph,
                &root_applicable,
                None,
                &active_locations,
                &mut projection,
            )?;
        } else if unsupported_regions.is_empty() && !is_explicitly_closed_object(&root_applicable) {
            return Err(CompileError::MissingProperties);
        }

        let fingerprint =
            fingerprint_compiled_definition(&controls, &objects, &arrays, graph, &used_resources);

        Ok(Self {
            controls,
            objects,
            arrays,
            unsupported_regions,
            capability_warnings,
            root_schema_locations,
            root_annotations,
            fingerprint,
        })
    }

    pub fn fingerprint(&self) -> DefinitionFingerprint {
        self.fingerprint
    }

    pub(crate) fn fingerprint_bytes(&self) -> &[u8; 32] {
        &self.fingerprint.0
    }

    pub fn controls(&self) -> impl Iterator<Item = ControlDefinitionView<'_>> {
        self.controls
            .iter()
            .map(|control| ControlDefinitionView { control })
    }

    pub fn objects(&self) -> impl Iterator<Item = ObjectDefinitionView<'_>> {
        self.objects
            .iter()
            .map(|object| ObjectDefinitionView { object })
    }

    pub fn arrays(&self) -> impl Iterator<Item = ArrayDefinitionView<'_>> {
        self.arrays
            .iter()
            .map(|array| ArrayDefinitionView { array })
    }

    pub fn array(&self, binding: &str) -> Option<ArrayDefinitionView<'_>> {
        self.arrays
            .iter()
            .find(|array| array.binding == binding)
            .map(|array| ArrayDefinitionView { array })
    }

    pub fn object_creation_seed(&self, binding: &str) -> Option<&Value> {
        self.objects
            .iter()
            .find(|object| object.binding == binding)
            .map(|object| &object.creation_seed)
    }

    pub fn unsupported_regions(&self) -> impl Iterator<Item = UnsupportedRegionView<'_>> {
        self.unsupported_regions
            .iter()
            .map(|region| UnsupportedRegionView { region })
    }

    pub fn capability_findings(&self) -> impl Iterator<Item = ProjectionFindingView<'_>> {
        self.capability_warnings
            .iter()
            .map(|(binding, finding)| ProjectionFindingView {
                finding,
                binding,
                blocking: false,
            })
            .chain(self.unsupported_regions.iter().flat_map(|region| {
                region.findings.iter().map(|finding| ProjectionFindingView {
                    finding,
                    binding: &region.binding,
                    blocking: true,
                })
            }))
    }

    pub fn root_schema_locations(&self) -> impl Iterator<Item = (&str, &str)> {
        self.root_schema_locations
            .iter()
            .map(|location| (location.resource.as_str(), location.pointer.as_str()))
    }

    pub fn root_annotations(&self) -> &DataSchemaAnnotations {
        &self.root_annotations
    }

    pub fn create_form(&self, form_data: Value) -> Result<Form, CreateFormError> {
        if !form_data.is_object() {
            return Err(CreateFormError::FormDataMustBeObject);
        }

        let controls = self
            .controls
            .iter()
            .cloned()
            .map(ControlState::new)
            .collect();
        let mut next_item_identity = 1;
        let arrays: Vec<ArrayState> = self
            .arrays
            .iter()
            .cloned()
            .map(|definition| {
                let items =
                    fresh_array_items_for_data(&definition, &form_data, &mut next_item_identity);
                ArrayState { definition, items }
            })
            .collect();
        let baseline_array_identities = arrays
            .iter()
            .map(|array: &ArrayState| array.items.iter().map(|item| item.identity).collect())
            .collect();
        let baseline = form_data.clone();

        Ok(Form {
            controls,
            arrays,
            next_item_identity,
            baseline_array_identities,
            definition_fingerprint: self.fingerprint,
            baseline,
            form_data,
            external_finding_batches: Vec::new(),
            submission_attempted: false,
            data_revision: 0,
            state_revision: 0,
        })
    }
}

#[derive(Clone)]
struct LocatedSchema<'a> {
    schema: &'a Value,
    document_index: usize,
    resource: String,
    pointer: PointerBuf,
    document_pointer: PointerBuf,
    resource_root: PointerBuf,
    all_of_memberships: Vec<AllOfMembership>,
    reference_path: Vec<SchemaIdentity>,
}

type SchemaIdentity = (usize, String);

#[derive(Clone, PartialEq, Eq)]
struct AllOfOrigin {
    resource: String,
    keyword_location: PointerBuf,
    branch_count: usize,
}

#[derive(Clone, PartialEq, Eq)]
struct AllOfMembership {
    origin: AllOfOrigin,
    branch_index: Option<usize>,
}

fn located_from_graph(location: GraphLocation<'_>) -> LocatedSchema<'_> {
    let identity = (
        location.document_index,
        location.document_pointer.to_string(),
    );
    LocatedSchema {
        schema: location.schema,
        document_index: location.document_index,
        resource: location.resource,
        pointer: location.pointer,
        document_pointer: location.document_pointer,
        resource_root: location.resource_root,
        all_of_memberships: Vec::new(),
        reference_path: vec![identity],
    }
}

fn schema_identity(located: &LocatedSchema<'_>) -> SchemaIdentity {
    (located.document_index, located.document_pointer.to_string())
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum ProjectedKind {
    String,
    Number,
    Integer,
    Boolean,
    Null,
    Object,
    Array,
}

struct ProjectionState<'a> {
    controls: &'a mut Vec<ControlDefinition>,
    objects: &'a mut Vec<ObjectDefinition>,
    arrays: &'a mut Vec<ArrayDefinition>,
    unsupported_regions: &'a mut Vec<UnsupportedRegion>,
    capability_warnings: &'a mut Vec<(PointerBuf, UnsupportedFinding)>,
    used_resources: &'a mut BTreeSet<String>,
    traversal: &'a mut ProjectionTraversal,
    inside_array_template: bool,
}

fn compile_properties<'a>(
    graph: &'a ResourceGraph,
    applicable: &[LocatedSchema<'a>],
    parent_binding: Option<&PointerBuf>,
    active_locations: &BTreeSet<SchemaIdentity>,
    projection: &mut ProjectionState<'_>,
) -> Result<(), CompileError> {
    let property_names = applicable
        .iter()
        .filter_map(|located| located.schema.get("properties").and_then(Value::as_object))
        .flat_map(|properties| properties.keys().cloned())
        .collect::<BTreeSet<_>>();
    if property_names.is_empty()
        && !applicable
            .iter()
            .any(|located| located.schema.get("properties").is_some())
    {
        if is_explicitly_closed_object(applicable) {
            return Ok(());
        }
        return Err(CompileError::MissingProperties);
    }

    let required = applicable
        .iter()
        .filter_map(|located| located.schema.get("required").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect::<HashSet<_>>();

    for property_name in property_names {
        projection.traversal.visit()?;
        let binding = append_pointer(parent_binding, [property_name.as_str()]);
        let mut child_schemas = Vec::new();
        for located in applicable {
            let Some(property_schema) = located
                .schema
                .get("properties")
                .and_then(Value::as_object)
                .and_then(|properties| properties.get(&property_name))
            else {
                continue;
            };
            let pointer = append_pointer(Some(&located.pointer), ["properties", &property_name]);
            let document_pointer = append_pointer(
                Some(&located.document_pointer),
                ["properties", &property_name],
            );
            let mut child = located_from_graph(
                graph
                    .normalize_location(GraphLocation {
                        schema: property_schema,
                        document_index: located.document_index,
                        resource: located.resource.clone(),
                        pointer,
                        document_pointer,
                        resource_root: located.resource_root.clone(),
                    })
                    .map_err(|_| CompileError::UnsupportedReference("$id".to_owned()))?,
            );
            child.all_of_memberships = located.all_of_memberships.clone();
            projection.used_resources.insert(child.resource.clone());
            child_schemas.push(child);
        }
        let child_expansion = expand_references(
            graph,
            child_schemas,
            active_locations,
            projection.used_resources,
            projection.traversal,
        )?;
        let child_schemas = child_expansion.schemas;
        let (title, title_warning) = text_annotation(&child_schemas, "title");
        let (help, description_warning) = text_annotation(&child_schemas, "description");
        let annotations = data_schema_annotations(&child_schemas);
        let label = title.unwrap_or_else(|| property_name.clone());
        for warning in [description_warning, title_warning].into_iter().flatten() {
            projection
                .capability_warnings
                .push((binding.clone(), warning));
        }
        let presentation = NodePresentation {
            label,
            help,
            annotations,
        };
        let schema_locations = schema_locations(&child_schemas);
        let is_required = required.contains(property_name.as_str());

        let kind = infer_kind(&child_schemas);
        let mut unsupported_findings = child_expansion.recursive_findings;
        let choices = match scalar_choices(&child_schemas, kind) {
            Ok(choices) => choices,
            Err(findings) => {
                unsupported_findings.extend(findings);
                None
            }
        };
        let kind_conflicts = if choices.is_some() {
            Vec::new()
        } else {
            all_of_kind_conflicts(&child_schemas)
        };
        unsupported_findings.extend(deferred_shape_findings(
            graph,
            &child_schemas,
            kind,
            choices.is_some(),
        ));
        if kind == Some(ProjectedKind::Object) {
            unsupported_findings.extend(dynamic_object_map_findings(&child_schemas));
        }

        for origin in kind_conflicts {
            unsupported_findings.push(all_of_unsupported_finding(origin, "incompatible-kind"));
        }
        if !unsupported_findings.is_empty() {
            projection.unsupported_regions.push(unsupported_region(
                binding,
                parent_binding.cloned(),
                schema_locations,
                presentation,
                is_required,
                unsupported_findings,
            ));
            continue;
        }

        match (choices, kind) {
            (Some(choices), _) => {
                let accepts_null = choices.values.iter().any(Value::is_null);
                let kind = if choices.null_only {
                    ControlKind::Null
                } else if choices.selectable {
                    ControlKind::Choice
                } else {
                    ControlKind::Constant
                };
                let creation_seed =
                    scalar_creation_seed(kind, &choices.values, &presentation.annotations);
                projection.controls.push(ControlDefinition {
                    binding,
                    parent_binding: parent_binding.cloned(),
                    kind,
                    choices: choices.values,
                    accepts_null,
                    schema_locations,
                    presentation,
                    creation_seed,
                    required: is_required,
                });
            }
            (
                None,
                Some(
                    kind @ (ProjectedKind::String
                    | ProjectedKind::Number
                    | ProjectedKind::Integer
                    | ProjectedKind::Boolean),
                ),
            ) => {
                let kind = match kind {
                    ProjectedKind::String => ControlKind::String,
                    ProjectedKind::Number => ControlKind::Number,
                    ProjectedKind::Integer => ControlKind::Integer,
                    ProjectedKind::Boolean => ControlKind::Boolean,
                    ProjectedKind::Null | ProjectedKind::Object | ProjectedKind::Array => {
                        unreachable!()
                    }
                };
                let creation_seed = scalar_creation_seed(kind, &[], &presentation.annotations);
                projection.controls.push(ControlDefinition {
                    binding,
                    parent_binding: parent_binding.cloned(),
                    kind,
                    choices: Vec::new(),
                    accepts_null: applicable_accepts_null(&child_schemas),
                    schema_locations,
                    presentation,
                    creation_seed,
                    required: is_required,
                });
            }
            (None, Some(ProjectedKind::Object)) => {
                projection
                    .capability_warnings
                    .extend(open_object_warnings(&child_schemas, binding.clone()));
                projection.objects.push(ObjectDefinition {
                    binding: binding.clone(),
                    parent_binding: parent_binding.cloned(),
                    schema_locations,
                    presentation,
                    required: is_required,
                    creation_seed: object_creation_seed(&child_schemas),
                });
                let mut child_active_locations = active_locations.clone();
                child_active_locations.extend(child_schemas.iter().map(schema_identity));
                compile_properties(
                    graph,
                    &child_schemas,
                    Some(&binding),
                    &child_active_locations,
                    projection,
                )?;
            }
            (None, Some(ProjectedKind::Array)) => {
                if projection.inside_array_template {
                    let origin = child_schemas
                        .first()
                        .expect("an array property has an applicable schema");
                    unsupported_findings.push(UnsupportedFinding {
                        code: "structure.array.nested",
                        keyword_location: origin.pointer.clone(),
                        resource: origin.resource.clone(),
                        parameters: serde_json::json!({}),
                    });
                    projection.unsupported_regions.push(unsupported_region(
                        binding,
                        parent_binding.cloned(),
                        schema_locations,
                        presentation,
                        is_required,
                        unsupported_findings,
                    ));
                } else {
                    match compile_homogeneous_array(
                        graph,
                        &child_schemas,
                        binding.clone(),
                        parent_binding.cloned(),
                        schema_locations.clone(),
                        presentation.clone(),
                        is_required,
                        active_locations,
                        projection.used_resources,
                        projection.capability_warnings,
                        projection.traversal,
                    )? {
                        Ok(array) => projection.arrays.push(array),
                        Err(findings) => projection.unsupported_regions.push(unsupported_region(
                            binding,
                            parent_binding.cloned(),
                            schema_locations,
                            presentation,
                            is_required,
                            findings,
                        )),
                    }
                }
            }
            (None, Some(ProjectedKind::Null)) => unreachable!("null-only controls have a choice"),
            (None, None) => {
                let location = child_schemas
                    .first()
                    .expect("declared properties always have an applicable schema");
                projection.unsupported_regions.push(unsupported_region(
                    binding,
                    parent_binding.cloned(),
                    schema_locations,
                    presentation,
                    is_required,
                    vec![unsupported_control_finding(&child_schemas, location)],
                ));
            }
        }
    }
    Ok(())
}

fn is_explicitly_closed_object(applicable: &[LocatedSchema<'_>]) -> bool {
    applicable.iter().any(|located| {
        located
            .schema
            .get("additionalProperties")
            .and_then(Value::as_bool)
            == Some(false)
    })
}

#[allow(clippy::too_many_arguments)]
fn compile_homogeneous_array<'a>(
    graph: &'a ResourceGraph,
    applicable: &[LocatedSchema<'a>],
    binding: PointerBuf,
    parent_binding: Option<PointerBuf>,
    array_schema_locations: Vec<SchemaLocationDefinition>,
    presentation: NodePresentation,
    required: bool,
    active_locations: &BTreeSet<SchemaIdentity>,
    used_resources: &mut BTreeSet<String>,
    capability_warnings: &mut Vec<(PointerBuf, UnsupportedFinding)>,
    traversal: &mut ProjectionTraversal,
) -> Result<Result<ArrayDefinition, Vec<UnsupportedFinding>>, CompileError> {
    let mut item_schemas = Vec::new();
    for located in applicable {
        let Some(item_schema) = located.schema.get("items") else {
            continue;
        };
        let item = located_subschema(graph, located, item_schema, &["items"]);
        used_resources.insert(item.resource.clone());
        item_schemas.push(item);
    }
    if item_schemas.is_empty() {
        if let Some(origin) = first_keyword_origin(applicable, "unevaluatedItems") {
            return Ok(Err(vec![UnsupportedFinding {
                code: "unevaluated.items.shape",
                keyword_location: append_pointer(Some(&origin.pointer), ["unevaluatedItems"]),
                resource: origin.resource.clone(),
                parameters: serde_json::json!({}),
            }]));
        }
        let origin = applicable
            .first()
            .expect("an array property has an applicable schema");
        return Ok(Err(vec![UnsupportedFinding {
            code: "structure.array.homogeneous-scalar",
            keyword_location: origin.pointer.clone(),
            resource: origin.resource.clone(),
            parameters: serde_json::json!({ "reason": "missing-items" }),
        }]));
    }

    let expansion = expand_references(
        graph,
        item_schemas,
        active_locations,
        used_resources,
        traversal,
    )?;
    let item_schemas = expansion.schemas;
    let kind = infer_kind(&item_schemas);
    let mut findings = expansion.recursive_findings;
    let choices = match scalar_choices(&item_schemas, kind) {
        Ok(choices) => choices,
        Err(choice_findings) => {
            findings.extend(choice_findings);
            None
        }
    };
    findings.extend(deferred_shape_findings(
        graph,
        &item_schemas,
        kind,
        choices.is_some(),
    ));
    if kind == Some(ProjectedKind::Object) {
        findings.extend(dynamic_object_map_findings(&item_schemas));
        if !findings.is_empty() {
            return Ok(Err(findings));
        }

        let (item_title, item_title_warning) = text_annotation(&item_schemas, "title");
        let (item_help, item_description_warning) = text_annotation(&item_schemas, "description");
        let item_presentation = NodePresentation {
            label: item_title.unwrap_or_else(|| format!("{} item", presentation.label)),
            help: item_help,
            annotations: data_schema_annotations(&item_schemas),
        };
        for warning in [item_description_warning, item_title_warning]
            .into_iter()
            .flatten()
        {
            capability_warnings.push((binding.clone(), warning));
        }
        capability_warnings.extend(open_object_warnings(&item_schemas, binding.clone()));

        let creation_seed = object_creation_seed(&item_schemas);
        let mut controls = Vec::new();
        let mut objects = vec![ObjectDefinition {
            binding: PointerBuf::new(),
            parent_binding: None,
            schema_locations: schema_locations(&item_schemas),
            presentation: item_presentation,
            required: true,
            creation_seed: creation_seed.clone(),
        }];
        let mut arrays = Vec::new();
        let mut unsupported_regions = Vec::new();
        let mut template_warnings = Vec::new();
        let mut child_active_locations = active_locations.clone();
        child_active_locations.extend(item_schemas.iter().map(schema_identity));
        let root_binding = PointerBuf::new();
        {
            let mut projection = ProjectionState {
                controls: &mut controls,
                objects: &mut objects,
                arrays: &mut arrays,
                unsupported_regions: &mut unsupported_regions,
                capability_warnings: &mut template_warnings,
                used_resources,
                traversal,
                inside_array_template: true,
            };
            compile_properties(
                graph,
                &item_schemas,
                Some(&root_binding),
                &child_active_locations,
                &mut projection,
            )?;
        }
        if !unsupported_regions.is_empty() {
            return Ok(Err(unsupported_regions
                .into_iter()
                .flat_map(|region| region.findings)
                .collect()));
        }
        debug_assert!(
            arrays.is_empty(),
            "nested arrays are rejected during projection"
        );
        capability_warnings.extend(
            template_warnings
                .into_iter()
                .map(|(_, finding)| (binding.clone(), finding)),
        );

        let (min_items, max_items) = array_length_bounds(applicable);
        return Ok(Ok(ArrayDefinition {
            binding,
            parent_binding,
            schema_locations: array_schema_locations,
            presentation,
            required,
            creation_seed: array_creation_seed(applicable),
            min_items,
            max_items,
            item_template: ArrayItemTemplate {
                controls,
                objects,
                creation_seed,
            },
        }));
    }
    let (item_kind, item_choices, accepts_null) = match (choices, kind) {
        (Some(choices), _) => {
            let accepts_null = choices.values.iter().any(Value::is_null);
            let kind = if choices.null_only {
                ControlKind::Null
            } else if choices.selectable {
                ControlKind::Choice
            } else {
                ControlKind::Constant
            };
            (kind, choices.values, accepts_null)
        }
        (
            None,
            Some(
                kind @ (ProjectedKind::String
                | ProjectedKind::Number
                | ProjectedKind::Integer
                | ProjectedKind::Boolean),
            ),
        ) => (
            match kind {
                ProjectedKind::String => ControlKind::String,
                ProjectedKind::Number => ControlKind::Number,
                ProjectedKind::Integer => ControlKind::Integer,
                ProjectedKind::Boolean => ControlKind::Boolean,
                _ => unreachable!(),
            },
            Vec::new(),
            applicable_accepts_null(&item_schemas),
        ),
        _ => {
            if findings.is_empty() {
                let origin = item_schemas
                    .first()
                    .expect("homogeneous array items have an applicable schema");
                findings.push(UnsupportedFinding {
                    code: match kind {
                        Some(ProjectedKind::Array) => "structure.array.nested",
                        Some(ProjectedKind::Object) => unreachable!(),
                        _ => "structure.array.homogeneous-scalar",
                    },
                    keyword_location: origin.pointer.clone(),
                    resource: origin.resource.clone(),
                    parameters: serde_json::json!({}),
                });
            }
            return Ok(Err(findings));
        }
    };
    if !findings.is_empty() {
        return Ok(Err(findings));
    }

    let (item_title, _) = text_annotation(&item_schemas, "title");
    let (item_help, _) = text_annotation(&item_schemas, "description");
    let item_presentation = NodePresentation {
        label: item_title.unwrap_or_else(|| format!("{} item", presentation.label)),
        help: item_help,
        annotations: data_schema_annotations(&item_schemas),
    };
    let creation_seed = array_item_creation_seed(
        item_kind,
        &item_choices,
        accepts_null,
        &item_presentation.annotations,
    );
    let item = ControlDefinition {
        binding: PointerBuf::new(),
        parent_binding: None,
        kind: item_kind,
        choices: item_choices,
        accepts_null,
        schema_locations: schema_locations(&item_schemas),
        presentation: item_presentation,
        creation_seed,
        required: true,
    };
    let (min_items, max_items) = array_length_bounds(applicable);
    Ok(Ok(ArrayDefinition {
        binding,
        parent_binding,
        schema_locations: array_schema_locations,
        presentation,
        required,
        creation_seed: array_creation_seed(applicable),
        min_items,
        max_items,
        item_template: ArrayItemTemplate {
            creation_seed: item
                .creation_seed
                .clone()
                .expect("supported scalar array items have a creation seed"),
            controls: vec![item],
            objects: Vec::new(),
        },
    }))
}

fn array_length_bounds(applicable: &[LocatedSchema<'_>]) -> (Option<usize>, Option<usize>) {
    let min_items = applicable
        .iter()
        .filter_map(|located| {
            located
                .schema
                .get("minItems")
                .and_then(nonnegative_integer_as_usize)
        })
        .max();
    let max_items = applicable
        .iter()
        .filter_map(|located| {
            located
                .schema
                .get("maxItems")
                .and_then(nonnegative_integer_as_usize)
        })
        .min();
    (min_items, max_items)
}

fn nonnegative_integer_as_usize(value: &Value) -> Option<usize> {
    let input = value.as_number()?.to_string();
    let mut normal = number_normal_form(&input)?;
    if normal.negative || normal.scale < BigInt::from(0_u8) {
        return None;
    }
    if normal.significant_digits == "0" {
        return Some(0);
    }

    let maximum_digits = usize::MAX.to_string().len();
    if normal.significant_digits.len() > maximum_digits {
        return Some(usize::MAX);
    }
    let available_digits = maximum_digits - normal.significant_digits.len();
    if normal.scale > BigInt::from(available_digits) {
        return Some(usize::MAX);
    }
    let trailing_zeros = normal.scale.to_string().parse::<usize>().ok()?;
    normal
        .significant_digits
        .extend(std::iter::repeat_n('0', trailing_zeros));
    Some(normal.significant_digits.parse().unwrap_or(usize::MAX))
}

fn append_relative_pointer(root: &PointerBuf, relative: &PointerBuf) -> PointerBuf {
    PointerBuf::parse(format!("{}{}", root.as_str(), relative.as_str()))
        .expect("joining valid JSON Pointers produces a valid JSON Pointer")
}

fn open_object_warnings(
    applicable: &[LocatedSchema<'_>],
    binding: PointerBuf,
) -> Vec<(PointerBuf, UnsupportedFinding)> {
    if !applicable
        .iter()
        .any(|located| located.schema.get("properties").is_some())
    {
        return Vec::new();
    }
    let closed = applicable
        .iter()
        .any(|located| located.schema.get("additionalProperties") == Some(&Value::Bool(false)));

    let mut warnings = Vec::new();
    if !closed
        && let Some(origin) = applicable
            .iter()
            .filter(|located| {
                matches!(
                    located.schema.get("additionalProperties"),
                    Some(Value::Object(_))
                )
            })
            .min_by(|left, right| {
                (&left.resource, left.pointer.as_str())
                    .cmp(&(&right.resource, right.pointer.as_str()))
            })
    {
        warnings.push((
            binding.clone(),
            UnsupportedFinding {
                code: "applicator.additional-properties.schema-projection",
                keyword_location: append_pointer(Some(&origin.pointer), ["additionalProperties"]),
                resource: origin.resource.clone(),
                parameters: serde_json::json!({}),
            },
        ));
    }
    if let Some(origin) = applicable
        .iter()
        .filter(|located| located.schema.get("patternProperties").is_some())
        .min_by(|left, right| {
            (&left.resource, left.pointer.as_str()).cmp(&(&right.resource, right.pointer.as_str()))
        })
    {
        warnings.push((
            binding.clone(),
            UnsupportedFinding {
                code: "applicator.pattern-properties.fixed-projection",
                keyword_location: append_pointer(Some(&origin.pointer), ["patternProperties"]),
                resource: origin.resource.clone(),
                parameters: serde_json::json!({}),
            },
        ));
    }
    if closed {
        return warnings;
    }

    let has_schema_additional_properties = applicable.iter().any(|located| {
        matches!(
            located.schema.get("additionalProperties"),
            Some(Value::Object(_))
        )
    });
    if has_schema_additional_properties {
        return warnings;
    }
    let explicit = applicable
        .iter()
        .filter(|located| located.schema.get("additionalProperties") == Some(&Value::Bool(true)))
        .min_by(|left, right| {
            (&left.resource, left.pointer.as_str()).cmp(&(&right.resource, right.pointer.as_str()))
        });
    let (origin, implicit) = if let Some(origin) = explicit {
        (origin, false)
    } else {
        let Some(origin) = applicable
            .iter()
            .filter(|located| located.schema.get("properties").is_some())
            .min_by(|left, right| {
                (&left.resource, left.pointer.as_str())
                    .cmp(&(&right.resource, right.pointer.as_str()))
            })
        else {
            return Vec::new();
        };
        (origin, true)
    };
    let keyword_location = if implicit {
        origin.pointer.clone()
    } else {
        append_pointer(Some(&origin.pointer), ["additionalProperties"])
    };

    warnings.push((
        binding,
        UnsupportedFinding {
            code: "applicator.additional-properties.open",
            keyword_location,
            resource: origin.resource.clone(),
            parameters: serde_json::json!({ "implicit": implicit }),
        },
    ));
    warnings
}

fn object_creation_seed(applicable: &[LocatedSchema<'_>]) -> Value {
    let mut eligible = Vec::new();
    for value in applicable
        .iter()
        .filter_map(|located| located.schema.get("default"))
        .filter(|value| value.is_object())
    {
        if !eligible
            .iter()
            .any(|existing| json_values_equal(existing, value))
        {
            eligible.push(value.clone());
        }
    }

    if eligible.len() == 1 {
        eligible[0].clone()
    } else {
        Value::Object(serde_json::Map::new())
    }
}

fn array_creation_seed(applicable: &[LocatedSchema<'_>]) -> Value {
    let mut eligible = Vec::new();
    for value in applicable
        .iter()
        .filter_map(|located| located.schema.get("default"))
        .filter(|value| value.is_array())
    {
        if !eligible
            .iter()
            .any(|existing| json_values_equal(existing, value))
        {
            eligible.push(value.clone());
        }
    }

    if eligible.len() == 1 {
        eligible.remove(0)
    } else {
        Value::Array(Vec::new())
    }
}

fn data_schema_annotations(applicable: &[LocatedSchema<'_>]) -> DataSchemaAnnotations {
    let mut examples = applicable
        .iter()
        .filter_map(|located| located.schema.get("examples").and_then(Value::as_array))
        .flatten()
        .fold(Vec::new(), |mut values, value| {
            push_unique_value(&mut values, value);
            values
        });
    sort_annotation_values(&mut examples);
    DataSchemaAnnotations {
        formats: string_annotations(applicable, "format"),
        defaults: value_annotations(applicable, "default"),
        deprecated: applicable
            .iter()
            .any(|located| located.schema.get("deprecated") == Some(&Value::Bool(true))),
        read_only: applicable
            .iter()
            .any(|located| located.schema.get("readOnly") == Some(&Value::Bool(true))),
        write_only: applicable
            .iter()
            .any(|located| located.schema.get("writeOnly") == Some(&Value::Bool(true))),
        examples,
        content_encodings: string_annotations(applicable, "contentEncoding"),
        content_media_types: string_annotations(applicable, "contentMediaType"),
        content_schemas: value_annotations(applicable, "contentSchema"),
    }
}

fn string_annotations(applicable: &[LocatedSchema<'_>], keyword: &str) -> Vec<String> {
    applicable
        .iter()
        .filter_map(|located| located.schema.get(keyword).and_then(Value::as_str))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn value_annotations(applicable: &[LocatedSchema<'_>], keyword: &str) -> Vec<Value> {
    let mut values = applicable
        .iter()
        .filter_map(|located| located.schema.get(keyword))
        .fold(Vec::new(), |mut values, value| {
            push_unique_value(&mut values, value);
            values
        });
    sort_annotation_values(&mut values);
    values
}

fn push_unique_value(values: &mut Vec<Value>, value: &Value) {
    if !values
        .iter()
        .any(|existing| json_values_equal(existing, value))
    {
        values.push(value.clone());
    }
}

fn sort_annotation_values(values: &mut [Value]) {
    values.sort_by_key(semantic_json_fingerprint);
}

fn scalar_creation_seed(
    kind: ControlKind,
    choices: &[Value],
    annotations: &DataSchemaAnnotations,
) -> Option<Value> {
    let eligible = annotations
        .defaults
        .iter()
        .filter(|value| !value.is_null() && scalar_value_is_compatible(kind, choices, value))
        .collect::<Vec<_>>();
    if eligible.len() == 1 {
        return Some(eligible[0].clone());
    }

    (kind != ControlKind::Null)
        .then(|| minimal_scalar_seed(kind, choices))
        .flatten()
}

fn array_item_creation_seed(
    kind: ControlKind,
    choices: &[Value],
    accepts_null: bool,
    annotations: &DataSchemaAnnotations,
) -> Option<Value> {
    let eligible = annotations
        .defaults
        .iter()
        .filter(|value| {
            value.is_null() && accepts_null || scalar_value_is_compatible(kind, choices, value)
        })
        .collect::<Vec<_>>();
    if eligible.len() == 1 {
        return Some(eligible[0].clone());
    }
    if kind == ControlKind::Null {
        return Some(Value::Null);
    }
    minimal_scalar_seed(kind, choices)
}

fn minimal_scalar_seed(kind: ControlKind, choices: &[Value]) -> Option<Value> {
    match kind {
        ControlKind::String => Some(Value::String(String::new())),
        ControlKind::Number | ControlKind::Integer => Some(serde_json::json!(0)),
        ControlKind::Boolean => Some(Value::Bool(false)),
        ControlKind::Choice | ControlKind::Constant => choices
            .iter()
            .find(|value| !value.is_null())
            .or_else(|| choices.first())
            .cloned(),
        ControlKind::Null => Some(Value::Null),
    }
}

fn scalar_value_is_compatible(kind: ControlKind, choices: &[Value], value: &Value) -> bool {
    match kind {
        ControlKind::String => value.is_string(),
        ControlKind::Number => value.is_number(),
        ControlKind::Integer => value
            .as_number()
            .is_some_and(|number| number_is_integer(&number.to_string())),
        ControlKind::Boolean => value.is_boolean(),
        ControlKind::Choice | ControlKind::Constant | ControlKind::Null => choices
            .iter()
            .any(|choice| json_values_equal(choice, value)),
    }
}

fn expand_references<'a>(
    graph: &'a ResourceGraph,
    schemas: Vec<LocatedSchema<'a>>,
    active_locations: &BTreeSet<SchemaIdentity>,
    used_resources: &mut BTreeSet<String>,
    traversal: &mut ProjectionTraversal,
) -> Result<SchemaExpansion<'a>, CompileError> {
    let mut expanded = Vec::new();
    let mut recursive_findings = BTreeMap::new();
    let mut pending = schemas;
    let mut seen = HashSet::new();
    while let Some(mut located) = pending.pop() {
        traversal.visit()?;
        let all_of = located.schema.get("allOf").and_then(Value::as_array);
        let all_of_origin = all_of.map(|branches| AllOfOrigin {
            resource: located.resource.clone(),
            keyword_location: append_pointer(Some(&located.pointer), ["allOf"]),
            branch_count: branches.len(),
        });
        if let Some(origin) = &all_of_origin {
            push_membership(
                &mut located.all_of_memberships,
                AllOfMembership {
                    origin: origin.clone(),
                    branch_index: None,
                },
            );
        }
        let mut membership_key = located
            .all_of_memberships
            .iter()
            .map(|membership| {
                (
                    membership.origin.resource.clone(),
                    membership.origin.keyword_location.to_string(),
                    membership.branch_index,
                )
            })
            .collect::<Vec<_>>();
        membership_key.sort();
        let key = (
            located.resource.clone(),
            located.pointer.to_string(),
            membership_key,
        );
        if !seen.insert(key) {
            continue;
        }
        if let Some(reference) = located.schema.get("$ref").and_then(Value::as_str) {
            let mut target = located_from_graph(
                graph
                    .resolve_reference(&located.resource, reference)
                    .map_err(|_| CompileError::UnsupportedReference(reference.to_owned()))?,
            );
            used_resources.insert(target.resource.clone());
            let target_identity = schema_identity(&target);
            if active_locations.contains(&target_identity)
                || located.reference_path.contains(&target_identity)
            {
                let keyword_location = append_pointer(Some(&located.pointer), ["$ref"]);
                recursive_findings.insert(
                    (located.resource.clone(), keyword_location.to_string()),
                    UnsupportedFinding {
                        code: "structure.recursive.projection",
                        keyword_location,
                        resource: located.resource.clone(),
                        parameters: serde_json::json!({}),
                    },
                );
            } else {
                target.all_of_memberships = located.all_of_memberships.clone();
                target.reference_path = located.reference_path.clone();
                target.reference_path.push(target_identity);
                pending.push(target);
            }
        }
        if let Some(reference) = located.schema.get("$dynamicRef").and_then(Value::as_str) {
            let target = graph
                .resolve_reference(&located.resource, reference)
                .map_err(|_| CompileError::UnsupportedReference(reference.to_owned()))?;
            include_validation_resources(graph, target, used_resources, traversal)?;
        }
        if let (Some(all_of), Some(origin)) = (all_of, all_of_origin) {
            for (branch_index, branch) in all_of.iter().enumerate() {
                let index = branch_index.to_string();
                let pointer = append_pointer(Some(&located.pointer), ["allOf", index.as_str()]);
                let document_pointer =
                    append_pointer(Some(&located.document_pointer), ["allOf", index.as_str()]);
                let mut branch = located_from_graph(
                    graph
                        .normalize_location(GraphLocation {
                            schema: branch,
                            document_index: located.document_index,
                            resource: located.resource.clone(),
                            pointer,
                            document_pointer,
                            resource_root: located.resource_root.clone(),
                        })
                        .map_err(|_| CompileError::UnsupportedReference("$id".to_owned()))?,
                );
                branch.all_of_memberships = located.all_of_memberships.clone();
                branch.reference_path = located.reference_path.clone();
                let branch_identity = schema_identity(&branch);
                if !branch.reference_path.contains(&branch_identity) {
                    branch.reference_path.push(branch_identity);
                }
                push_membership(
                    &mut branch.all_of_memberships,
                    AllOfMembership {
                        origin: origin.clone(),
                        branch_index: Some(branch_index),
                    },
                );
                used_resources.insert(branch.resource.clone());
                pending.push(branch);
            }
        }
        expanded.push(located);
    }
    expanded.sort_by(|left, right| {
        left.resource
            .cmp(&right.resource)
            .then_with(|| left.pointer.as_str().cmp(right.pointer.as_str()))
    });
    Ok(SchemaExpansion {
        schemas: expanded,
        recursive_findings: recursive_findings.into_values().collect(),
    })
}

struct SchemaExpansion<'a> {
    schemas: Vec<LocatedSchema<'a>>,
    recursive_findings: Vec<UnsupportedFinding>,
}

fn include_validation_resources(
    graph: &ResourceGraph,
    initial: GraphLocation<'_>,
    used_resources: &mut BTreeSet<String>,
    traversal: &mut ProjectionTraversal,
) -> Result<(), CompileError> {
    let mut pending = vec![initial];
    let mut visited = BTreeSet::new();
    while let Some(location) = pending.pop() {
        traversal.visit()?;
        let identity = (
            location.document_index,
            location.document_pointer.to_string(),
        );
        if !visited.insert(identity) {
            continue;
        }
        used_resources.insert(location.resource.clone());
        for keyword in ["$ref", "$dynamicRef"] {
            if let Some(reference) = location.schema.get(keyword).and_then(Value::as_str) {
                pending.push(
                    graph
                        .resolve_reference(&location.resource, reference)
                        .map_err(|_| CompileError::UnsupportedReference(reference.to_owned()))?,
                );
            }
        }
        pending.extend(
            graph
                .applicable_children(&location)
                .map_err(|_| CompileError::UnsupportedReference("$dynamicRef".to_owned()))?,
        );
    }
    Ok(())
}

struct ProjectionTraversal {
    maximum: usize,
    observed: usize,
}

impl ProjectionTraversal {
    fn new(maximum: usize) -> Self {
        Self {
            maximum,
            observed: 0,
        }
    }

    fn visit(&mut self) -> Result<(), CompileError> {
        self.observed = self.observed.saturating_add(1);
        if self.observed > self.maximum {
            return Err(CompileError::ResourceLimit(CompilationLimitError::new(
                CompilationLimitPhase::Projection,
                CompilationLimitDimension::Traversal,
                self.maximum,
                self.observed,
                String::new(),
            )));
        }
        Ok(())
    }
}

fn deferred_shape_findings(
    graph: &ResourceGraph,
    applicable: &[LocatedSchema<'_>],
    independent_kind: Option<ProjectedKind>,
    has_finite_choices: bool,
) -> Vec<UnsupportedFinding> {
    let mut findings = array_applicator_findings(applicable, "oneOf", "applicator.one-of");
    findings.extend(array_applicator_findings(
        applicable,
        "anyOf",
        "applicator.any-of",
    ));
    if array_may_apply(applicable) {
        findings.extend(array_applicator_findings(
            applicable,
            "prefixItems",
            "applicator.prefix-items",
        ));
    }

    if independent_kind.is_none() && !has_finite_choices {
        findings.extend(keyword_findings(
            applicable,
            "$dynamicRef",
            "core.dynamic-reference.shape",
        ));
    } else if independent_kind == Some(ProjectedKind::Object) {
        findings.extend(dynamic_object_shape_findings(graph, applicable));
    }
    findings.extend(structural_conditional_findings(
        graph,
        applicable,
        independent_kind,
    ));
    findings.extend(conditional_choice_findings(graph, applicable));
    findings.extend(structural_dependent_schema_findings(
        graph,
        applicable,
        independent_kind,
    ));
    if independent_kind.is_none() && !has_finite_choices {
        findings.extend(keyword_findings(applicable, "not", "applicator.not.shape"));
    }
    findings
}

fn unsupported_root_finding(
    applicable: &[LocatedSchema<'_>],
    kind: Option<ProjectedKind>,
    has_scalar_choices: bool,
) -> UnsupportedFinding {
    let location = applicable
        .first()
        .expect("the qualified root always has an applicable schema");
    let declared_kinds = declared_type_names(applicable);
    let (code, keyword) = if kind == Some(ProjectedKind::Array) {
        ("structure.root.array", Some("type"))
    } else if kind.is_some() || has_scalar_choices {
        ("structure.root.scalar", kind.is_some().then_some("type"))
    } else if declared_kinds.len() == 1 && declared_kinds.contains("array") {
        ("structure.root.array", Some("type"))
    } else if applicable.iter().any(|located| located.schema.is_boolean()) {
        ("core.boolean.unconstrained", None)
    } else if !declared_kinds.is_empty() {
        ("validation.type.ambiguous", Some("type"))
    } else {
        ("validation.type.unconstrained", None)
    };
    let origin = keyword
        .and_then(|keyword| first_keyword_origin(applicable, keyword))
        .unwrap_or(location);
    let keyword_location = keyword.map_or_else(
        || origin.pointer.clone(),
        |keyword| append_pointer(Some(&origin.pointer), [keyword]),
    );
    UnsupportedFinding {
        code,
        keyword_location,
        resource: origin.resource.clone(),
        parameters: serde_json::json!({}),
    }
}

fn unsupported_control_finding(
    applicable: &[LocatedSchema<'_>],
    location: &LocatedSchema<'_>,
) -> UnsupportedFinding {
    let (code, origin, keyword_location) =
        if applicable.iter().any(|located| located.schema.is_boolean()) {
            (
                "core.boolean.unconstrained",
                location,
                location.pointer.clone(),
            )
        } else if let Some(origin) = first_keyword_origin(applicable, "type") {
            (
                "validation.type.ambiguous",
                origin,
                append_pointer(Some(&origin.pointer), ["type"]),
            )
        } else {
            (
                "validation.type.unconstrained",
                location,
                location.pointer.clone(),
            )
        };
    UnsupportedFinding {
        code,
        keyword_location,
        resource: origin.resource.clone(),
        parameters: serde_json::json!({}),
    }
}

fn declared_type_names<'a>(applicable: &'a [LocatedSchema<'_>]) -> BTreeSet<&'a str> {
    applicable
        .iter()
        .filter_map(|located| located.schema.get("type"))
        .flat_map(|declared| {
            declared
                .as_array()
                .map(|kinds| kinds.iter().filter_map(Value::as_str).collect::<Vec<_>>())
                .unwrap_or_else(|| declared.as_str().into_iter().collect())
        })
        .collect()
}

fn first_keyword_origin<'a, 'schema>(
    applicable: &'a [LocatedSchema<'schema>],
    keyword: &str,
) -> Option<&'a LocatedSchema<'schema>> {
    applicable
        .iter()
        .filter(|located| located.schema.get(keyword).is_some())
        .min_by(|left, right| {
            (&left.resource, left.pointer.as_str()).cmp(&(&right.resource, right.pointer.as_str()))
        })
}

fn dynamic_object_map_findings(applicable: &[LocatedSchema<'_>]) -> Vec<UnsupportedFinding> {
    if applicable
        .iter()
        .any(|located| located.schema.get("properties").is_some())
    {
        return Vec::new();
    }
    let mut findings = Vec::new();
    let closed = applicable
        .iter()
        .any(|located| located.schema.get("additionalProperties") == Some(&Value::Bool(false)));
    if !closed {
        let explicit = applicable
            .iter()
            .filter(|located| {
                matches!(
                    located.schema.get("additionalProperties"),
                    Some(Value::Object(_)) | Some(Value::Bool(true))
                )
            })
            .min_by(|left, right| {
                (&left.resource, left.pointer.as_str())
                    .cmp(&(&right.resource, right.pointer.as_str()))
            });
        let origin = explicit.or_else(|| {
            applicable.iter().min_by(|left, right| {
                (&left.resource, left.pointer.as_str())
                    .cmp(&(&right.resource, right.pointer.as_str()))
            })
        });
        if let Some(origin) = origin {
            findings.push(UnsupportedFinding {
                code: "applicator.additional-properties.dynamic-map",
                keyword_location: if explicit.is_some() {
                    append_pointer(Some(&origin.pointer), ["additionalProperties"])
                } else {
                    origin.pointer.clone()
                },
                resource: origin.resource.clone(),
                parameters: serde_json::json!({ "implicit": explicit.is_none() }),
            });
        }
    }
    for (keyword, code) in [
        ("patternProperties", "applicator.pattern-properties.shape"),
        ("unevaluatedProperties", "unevaluated.properties.shape"),
    ] {
        if let Some(origin) = applicable
            .iter()
            .filter(|located| located.schema.get(keyword).is_some())
            .min_by(|left, right| {
                (&left.resource, left.pointer.as_str())
                    .cmp(&(&right.resource, right.pointer.as_str()))
            })
        {
            findings.push(UnsupportedFinding {
                code,
                keyword_location: append_pointer(Some(&origin.pointer), [keyword]),
                resource: origin.resource.clone(),
                parameters: serde_json::json!({}),
            });
        }
    }
    findings
}

fn structural_dependent_schema_findings(
    graph: &ResourceGraph,
    applicable: &[LocatedSchema<'_>],
    independent_kind: Option<ProjectedKind>,
) -> Vec<UnsupportedFinding> {
    let independent_properties = independent_property_kinds(graph, applicable);
    let independent_required = applicable
        .iter()
        .filter_map(|located| located.schema.get("required").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    let mut findings = BTreeMap::new();
    for located in applicable {
        let Some(dependencies) = located
            .schema
            .get("dependentSchemas")
            .and_then(Value::as_object)
        else {
            continue;
        };
        let structural = dependencies.iter().any(|(property, branch)| {
            let branch = located_subschema(
                graph,
                located,
                branch,
                &["dependentSchemas", property.as_str()],
            );
            branch_changes_projection(
                graph,
                &branch,
                independent_kind,
                &independent_properties,
                &independent_required,
                &mut BTreeSet::new(),
            )
        });
        if structural {
            let keyword_location = append_pointer(Some(&located.pointer), ["dependentSchemas"]);
            findings.insert(
                (located.resource.clone(), keyword_location.to_string()),
                UnsupportedFinding {
                    code: "applicator.dependent-schemas.structural",
                    keyword_location,
                    resource: located.resource.clone(),
                    parameters: serde_json::json!({}),
                },
            );
        }
    }
    findings.into_values().collect()
}

fn array_may_apply(applicable: &[LocatedSchema<'_>]) -> bool {
    let declarations = applicable
        .iter()
        .filter_map(|located| located.schema.get("type"))
        .collect::<Vec<_>>();
    declarations.is_empty()
        || declarations.iter().all(|declared| {
            declared.as_str() == Some("array")
                || declared
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind.as_str() == Some("array")))
        })
}

fn dynamic_object_shape_findings(
    graph: &ResourceGraph,
    applicable: &[LocatedSchema<'_>],
) -> Vec<UnsupportedFinding> {
    let mut findings = BTreeMap::new();
    let independent_properties = independent_property_kinds(graph, applicable);
    let independent_required = applicable
        .iter()
        .filter_map(|located| located.schema.get("required").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    for located in applicable {
        let Some(reference) = located.schema.get("$dynamicRef").and_then(Value::as_str) else {
            continue;
        };
        let target = graph
            .resolve_reference(&located.resource, reference)
            .expect("qualified dynamic references remain resolvable during projection");
        let target = located_from_graph(target);
        if !branch_changes_projection(
            graph,
            &target,
            Some(ProjectedKind::Object),
            &independent_properties,
            &independent_required,
            &mut BTreeSet::new(),
        ) {
            continue;
        }
        let keyword_location = append_pointer(Some(&located.pointer), ["$dynamicRef"]);
        findings.insert(
            (located.resource.clone(), keyword_location.to_string()),
            UnsupportedFinding {
                code: "core.dynamic-reference.shape",
                keyword_location,
                resource: located.resource.clone(),
                parameters: serde_json::json!({}),
            },
        );
    }
    findings.into_values().collect()
}

fn array_applicator_findings(
    applicable: &[LocatedSchema<'_>],
    keyword: &'static str,
    code: &'static str,
) -> Vec<UnsupportedFinding> {
    let mut findings = BTreeMap::new();
    for located in applicable {
        let Some(values) = located.schema.get(keyword).and_then(Value::as_array) else {
            continue;
        };
        let keyword_location = append_pointer(Some(&located.pointer), [keyword]);
        findings.insert(
            (located.resource.clone(), keyword_location.to_string()),
            UnsupportedFinding {
                code,
                keyword_location,
                resource: located.resource.clone(),
                parameters: if keyword == "prefixItems" {
                    serde_json::json!({ "itemCount": values.len() })
                } else {
                    serde_json::json!({ "branchCount": values.len() })
                },
            },
        );
    }
    findings.into_values().collect()
}

fn keyword_findings(
    applicable: &[LocatedSchema<'_>],
    keyword: &'static str,
    code: &'static str,
) -> Vec<UnsupportedFinding> {
    let mut findings = BTreeMap::new();
    for located in applicable {
        if located.schema.get(keyword).is_none() {
            continue;
        }
        let keyword_location = append_pointer(Some(&located.pointer), [keyword]);
        findings.insert(
            (located.resource.clone(), keyword_location.to_string()),
            UnsupportedFinding {
                code,
                keyword_location,
                resource: located.resource.clone(),
                parameters: serde_json::json!({}),
            },
        );
    }
    findings.into_values().collect()
}

fn structural_conditional_findings(
    graph: &ResourceGraph,
    applicable: &[LocatedSchema<'_>],
    independent_kind: Option<ProjectedKind>,
) -> Vec<UnsupportedFinding> {
    let mut findings = BTreeMap::new();
    let independent_properties = independent_property_kinds(graph, applicable);
    let independent_required = applicable
        .iter()
        .filter_map(|located| located.schema.get("required").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_str)
        .collect::<BTreeSet<_>>();
    for located in applicable {
        if located.schema.get("if").is_none() {
            continue;
        }
        let then_is_structural = located.schema.get("then").is_some_and(|branch| {
            let branch = located_subschema(graph, located, branch, &["then"]);
            branch_changes_projection(
                graph,
                &branch,
                independent_kind,
                &independent_properties,
                &independent_required,
                &mut BTreeSet::new(),
            )
        });
        let else_is_structural = located.schema.get("else").is_some_and(|branch| {
            let branch = located_subschema(graph, located, branch, &["else"]);
            branch_changes_projection(
                graph,
                &branch,
                independent_kind,
                &independent_properties,
                &independent_required,
                &mut BTreeSet::new(),
            )
        });
        let then_changes_properties = located.schema.get("then").is_some_and(|branch| {
            conditional_properties_change_projection(
                graph,
                located,
                branch,
                "then",
                independent_kind,
                &independent_properties,
            )
        });
        let else_changes_properties = located.schema.get("else").is_some_and(|branch| {
            conditional_properties_change_projection(
                graph,
                located,
                branch,
                "else",
                independent_kind,
                &independent_properties,
            )
        });
        if !then_is_structural && !else_is_structural {
            continue;
        }

        for membership in located
            .all_of_memberships
            .iter()
            .filter(|membership| membership.branch_index.is_some())
        {
            findings.insert(
                (
                    membership.origin.resource.clone(),
                    membership.origin.keyword_location.to_string(),
                    "applicator.all-of.conditional",
                ),
                UnsupportedFinding {
                    code: "applicator.all-of.conditional",
                    keyword_location: membership.origin.keyword_location.clone(),
                    resource: membership.origin.resource.clone(),
                    parameters: serde_json::json!({}),
                },
            );
        }

        for (keyword, code, is_structural) in [
            (
                "if",
                "applicator.if.structural",
                located.schema.get("if").is_some(),
            ),
            ("then", "applicator.then.structural", then_is_structural),
            ("else", "applicator.else.structural", else_is_structural),
        ] {
            if !is_structural {
                continue;
            }
            let keyword_location = append_pointer(Some(&located.pointer), [keyword]);
            findings.insert(
                (located.resource.clone(), keyword_location.to_string(), code),
                UnsupportedFinding {
                    code,
                    keyword_location,
                    resource: located.resource.clone(),
                    parameters: serde_json::json!({}),
                },
            );
        }
        for (branch, changes_properties) in [
            ("then", then_changes_properties),
            ("else", else_changes_properties),
        ] {
            if changes_properties {
                let keyword_location =
                    append_pointer(Some(&located.pointer), [branch, "properties"]);
                findings.insert(
                    (
                        located.resource.clone(),
                        keyword_location.to_string(),
                        "applicator.properties.conditional",
                    ),
                    UnsupportedFinding {
                        code: "applicator.properties.conditional",
                        keyword_location,
                        resource: located.resource.clone(),
                        parameters: serde_json::json!({ "branch": branch }),
                    },
                );
            }
        }
    }
    findings.into_values().collect()
}

fn conditional_properties_change_projection(
    graph: &ResourceGraph,
    parent: &LocatedSchema<'_>,
    branch: &Value,
    branch_keyword: &str,
    independent_kind: Option<ProjectedKind>,
    independent_properties: &BTreeMap<String, BTreeSet<ProjectedKind>>,
) -> bool {
    if independent_kind != Some(ProjectedKind::Object) {
        return false;
    }
    let branch = located_subschema(graph, parent, branch, &[branch_keyword]);
    branch
        .schema
        .get("properties")
        .and_then(Value::as_object)
        .is_some_and(|properties| {
            properties.iter().any(|(property, schema)| {
                let property_schema =
                    located_subschema(graph, &branch, schema, &["properties", property]);
                !independent_properties.contains_key(property)
                    || property_schema_changes_projection(
                        graph,
                        &property_schema,
                        independent_properties.get(property),
                        &mut BTreeSet::new(),
                    )
            })
        })
}

fn conditional_choice_findings(
    graph: &ResourceGraph,
    applicable: &[LocatedSchema<'_>],
) -> Vec<UnsupportedFinding> {
    let mut findings = BTreeMap::new();
    for located in applicable {
        if located.schema.get("if").is_none() {
            continue;
        }
        for branch_keyword in ["then", "else"] {
            let Some(branch) = located.schema.get(branch_keyword) else {
                continue;
            };
            let branch = located_subschema(graph, located, branch, &[branch_keyword]);
            collect_conditional_choice_findings(graph, branch, &mut findings);
        }
    }
    findings.into_values().collect()
}

fn collect_conditional_choice_findings<'a>(
    graph: &'a ResourceGraph,
    initial: LocatedSchema<'a>,
    findings: &mut BTreeMap<(String, String), UnsupportedFinding>,
) {
    let mut pending = vec![initial];
    let mut visited = BTreeSet::new();
    while let Some(located) = pending.pop() {
        let identity = schema_identity(&located);
        if !visited.insert(identity) {
            continue;
        }
        if located.schema.get("enum").is_some() {
            let keyword_location = append_pointer(Some(&located.pointer), ["enum"]);
            findings.insert(
                (located.resource.clone(), keyword_location.to_string()),
                UnsupportedFinding {
                    code: "validation.enum.conditional",
                    keyword_location,
                    resource: located.resource.clone(),
                    parameters: serde_json::json!({}),
                },
            );
        }
        if let Some(reference) = located.schema.get("$ref").and_then(Value::as_str) {
            let target = graph
                .resolve_reference(&located.resource, reference)
                .expect("qualified conditional references remain resolvable");
            pending.push(located_from_graph(target));
        }
        let Some(object) = located.schema.as_object() else {
            continue;
        };
        if let Some(properties) = object.get("properties").and_then(Value::as_object) {
            for (name, child) in properties {
                pending.push(located_subschema(
                    graph,
                    &located,
                    child,
                    &["properties", name],
                ));
            }
        }
        if let Some(branches) = object.get("allOf").and_then(Value::as_array) {
            for (index, child) in branches.iter().enumerate() {
                let index = index.to_string();
                pending.push(located_subschema(
                    graph,
                    &located,
                    child,
                    &["allOf", &index],
                ));
            }
        }
        if let Some(items) = object.get("items") {
            pending.push(located_subschema(graph, &located, items, &["items"]));
        }
        if object.get("if").is_some() {
            for keyword in ["then", "else"] {
                if let Some(child) = object.get(keyword) {
                    pending.push(located_subschema(graph, &located, child, &[keyword]));
                }
            }
        }
    }
}

fn independent_property_kinds(
    graph: &ResourceGraph,
    applicable: &[LocatedSchema<'_>],
) -> BTreeMap<String, BTreeSet<ProjectedKind>> {
    let mut properties = BTreeMap::<String, BTreeSet<ProjectedKind>>::new();
    for located in applicable {
        if let Some(schemas) = located.schema.get("properties").and_then(Value::as_object) {
            for (name, schema) in schemas {
                let child = located_subschema(graph, located, schema, &["properties", name]);
                properties
                    .entry(name.clone())
                    .or_default()
                    .extend(projected_kinds_from_schema(
                        graph,
                        &child,
                        &mut BTreeSet::new(),
                    ));
            }
        }
    }
    properties
}

fn branch_changes_projection(
    graph: &ResourceGraph,
    branch: &LocatedSchema<'_>,
    independent_kind: Option<ProjectedKind>,
    independent_properties: &BTreeMap<String, BTreeSet<ProjectedKind>>,
    independent_required: &BTreeSet<&str>,
    visited_references: &mut BTreeSet<SchemaIdentity>,
) -> bool {
    let Some(schema) = branch.schema.as_object() else {
        return false;
    };
    let adds_properties = independent_kind == Some(ProjectedKind::Object)
        && schema
            .get("properties")
            .and_then(Value::as_object)
            .is_some_and(|properties| {
                properties.iter().any(|(property, schema)| {
                    let property_schema =
                        located_subschema(graph, branch, schema, &["properties", property]);
                    let mut property_references = visited_references.clone();
                    !independent_properties.contains_key(property)
                        || property_schema_changes_projection(
                            graph,
                            &property_schema,
                            independent_properties.get(property),
                            &mut property_references,
                        )
                })
            });
    let changes_presence = independent_kind == Some(ProjectedKind::Object)
        && schema
            .get("required")
            .and_then(Value::as_array)
            .is_some_and(|required| {
                required
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|property| !independent_required.contains(property))
            });
    if adds_properties
        || changes_presence
        || (independent_kind.is_none()
            && (schema.contains_key("items") || schema.contains_key("prefixItems")))
        || (independent_kind == Some(ProjectedKind::Object)
            && schema.contains_key("dependentSchemas"))
        || (matches!(independent_kind, None | Some(ProjectedKind::Object))
            && schema.contains_key("$dynamicRef"))
        || schema.contains_key("oneOf")
        || schema.contains_key("anyOf")
        || schema.contains_key("enum")
        || schema.contains_key("const")
    {
        return true;
    }
    if let Some(declared) = schema.get("type") {
        let branch_kind = declared.as_str().and_then(projected_kind);
        if independent_kind.is_none() || branch_kind != independent_kind {
            return true;
        }
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let target = graph
            .resolve_reference(&branch.resource, reference)
            .expect("qualified references remain resolvable during projection");
        let target = located_from_graph(target);
        let identity = schema_identity(&target);
        if visited_references.insert(identity)
            && branch_changes_projection(
                graph,
                &target,
                independent_kind,
                independent_properties,
                independent_required,
                visited_references,
            )
        {
            return true;
        }
    }
    schema
        .get("allOf")
        .and_then(Value::as_array)
        .is_some_and(|branches| {
            branches.iter().enumerate().any(|(index, schema)| {
                let index = index.to_string();
                let nested = located_subschema(graph, branch, schema, &["allOf", &index]);
                branch_changes_projection(
                    graph,
                    &nested,
                    independent_kind,
                    independent_properties,
                    independent_required,
                    visited_references,
                )
            })
        })
        || (schema.get("if").is_some()
            && ["then", "else"].into_iter().any(|keyword| {
                schema.get(keyword).is_some_and(|nested| {
                    let nested = located_subschema(graph, branch, nested, &[keyword]);
                    branch_changes_projection(
                        graph,
                        &nested,
                        independent_kind,
                        independent_properties,
                        independent_required,
                        visited_references,
                    )
                })
            }))
}

fn property_schema_changes_projection(
    graph: &ResourceGraph,
    schema: &LocatedSchema<'_>,
    independent_kinds: Option<&BTreeSet<ProjectedKind>>,
    visited_references: &mut BTreeSet<SchemaIdentity>,
) -> bool {
    let Some(object) = schema.schema.as_object() else {
        return false;
    };
    if let Some(declared) = object.get("type") {
        let branch_kind = declared.as_str().and_then(projected_kind);
        if !matches!(
            (branch_kind, independent_kinds),
            (Some(branch_kind), Some(independent))
                if independent.len() == 1 && independent.contains(&branch_kind)
        ) {
            return true;
        }
    }
    if [
        "properties",
        "required",
        "items",
        "prefixItems",
        "dependentSchemas",
        "$dynamicRef",
        "oneOf",
        "anyOf",
        "enum",
        "const",
    ]
    .into_iter()
    .any(|keyword| object.contains_key(keyword))
    {
        return true;
    }
    if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
        let target = graph
            .resolve_reference(&schema.resource, reference)
            .expect("qualified references remain resolvable during projection");
        let target = located_from_graph(target);
        let identity = schema_identity(&target);
        if visited_references.insert(identity)
            && property_schema_changes_projection(
                graph,
                &target,
                independent_kinds,
                visited_references,
            )
        {
            return true;
        }
    }
    object
        .get("allOf")
        .and_then(Value::as_array)
        .is_some_and(|branches| {
            branches.iter().enumerate().any(|(index, branch)| {
                let index = index.to_string();
                let branch = located_subschema(graph, schema, branch, &["allOf", &index]);
                property_schema_changes_projection(
                    graph,
                    &branch,
                    independent_kinds,
                    visited_references,
                )
            })
        })
}

fn projected_kinds_from_schema(
    graph: &ResourceGraph,
    schema: &LocatedSchema<'_>,
    visited_references: &mut BTreeSet<SchemaIdentity>,
) -> BTreeSet<ProjectedKind> {
    let mut kinds = schema
        .schema
        .get("type")
        .and_then(Value::as_str)
        .and_then(projected_kind)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if let Some(reference) = schema.schema.get("$ref").and_then(Value::as_str) {
        let target = graph
            .resolve_reference(&schema.resource, reference)
            .expect("qualified references remain resolvable during projection");
        let target = located_from_graph(target);
        if visited_references.insert(schema_identity(&target)) {
            kinds.extend(projected_kinds_from_schema(
                graph,
                &target,
                visited_references,
            ));
        }
    }
    if let Some(branches) = schema.schema.get("allOf").and_then(Value::as_array) {
        for (index, branch) in branches.iter().enumerate() {
            let index = index.to_string();
            let branch = located_subschema(graph, schema, branch, &["allOf", &index]);
            kinds.extend(projected_kinds_from_schema(
                graph,
                &branch,
                visited_references,
            ));
        }
    }
    kinds
}

fn located_subschema<'a>(
    graph: &'a ResourceGraph,
    parent: &LocatedSchema<'a>,
    schema: &'a Value,
    tokens: &[&str],
) -> LocatedSchema<'a> {
    let pointer = append_pointer(Some(&parent.pointer), tokens.iter().copied());
    let document_pointer = append_pointer(Some(&parent.document_pointer), tokens.iter().copied());
    let mut located = located_from_graph(
        graph
            .normalize_location(GraphLocation {
                schema,
                document_index: parent.document_index,
                resource: parent.resource.clone(),
                pointer,
                document_pointer,
                resource_root: parent.resource_root.clone(),
            })
            .expect("qualified subschemas remain normalized during projection"),
    );
    located.all_of_memberships = parent.all_of_memberships.clone();
    located
}

fn push_membership(memberships: &mut Vec<AllOfMembership>, membership: AllOfMembership) {
    if !memberships.contains(&membership) {
        memberships.push(membership);
    }
}

fn text_annotation(
    applicable: &[LocatedSchema<'_>],
    keyword: &'static str,
) -> (Option<String>, Option<UnsupportedFinding>) {
    let values = applicable
        .iter()
        .filter_map(|located| located.schema.get(keyword).and_then(Value::as_str))
        .collect::<BTreeSet<_>>();
    if values.len() <= 1 {
        return (values.first().map(|value| (*value).to_owned()), None);
    }

    let origin = applicable
        .iter()
        .filter(|located| {
            located
                .schema
                .get(keyword)
                .and_then(Value::as_str)
                .is_some()
        })
        .min_by(|left, right| {
            (&left.resource, left.pointer.as_str()).cmp(&(&right.resource, right.pointer.as_str()))
        })
        .expect("conflicting annotations have an origin");
    (
        None,
        Some(UnsupportedFinding {
            code: "annotation.conflict",
            keyword_location: append_pointer(Some(&origin.pointer), [keyword]),
            resource: origin.resource.clone(),
            parameters: serde_json::json!({
                "keyword": keyword,
                "values": values.into_iter().collect::<Vec<_>>(),
            }),
        }),
    )
}

fn all_of_kind_conflicts(applicable: &[LocatedSchema<'_>]) -> Vec<AllOfOrigin> {
    leaf_conflict_origins(
        applicable,
        branch_contributing_origins(applicable, |located| located.schema.get("type").is_some())
            .into_iter()
            .filter(|origin| infer_kind(&schemas_for_origin(applicable, origin)).is_none())
            .collect(),
    )
}

fn branch_contributing_origins(
    applicable: &[LocatedSchema<'_>],
    has_assertion: impl Fn(&LocatedSchema<'_>) -> bool,
) -> Vec<AllOfOrigin> {
    let mut origins = applicable
        .iter()
        .flat_map(|located| {
            located
                .all_of_memberships
                .iter()
                .filter(|membership| membership.branch_index.is_some() && has_assertion(located))
                .map(|membership| membership.origin.clone())
        })
        .collect::<Vec<_>>();
    origins.sort_by(|left, right| {
        left.resource.cmp(&right.resource).then_with(|| {
            left.keyword_location
                .as_str()
                .cmp(right.keyword_location.as_str())
        })
    });
    origins.dedup();
    origins
}

fn schemas_for_origin<'a>(
    applicable: &[LocatedSchema<'a>],
    origin: &AllOfOrigin,
) -> Vec<LocatedSchema<'a>> {
    applicable
        .iter()
        .filter(|located| {
            located
                .all_of_memberships
                .iter()
                .any(|membership| &membership.origin == origin)
        })
        .cloned()
        .collect()
}

fn leaf_conflict_origins(
    applicable: &[LocatedSchema<'_>],
    candidates: Vec<AllOfOrigin>,
) -> Vec<AllOfOrigin> {
    let mut leaves = candidates
        .iter()
        .filter(|candidate| {
            !candidates
                .iter()
                .any(|other| candidate != &other && origin_precedes(candidate, other, applicable))
        })
        .cloned()
        .collect::<Vec<_>>();
    leaves.sort_by(|left, right| {
        left.resource.cmp(&right.resource).then_with(|| {
            left.keyword_location
                .as_str()
                .cmp(right.keyword_location.as_str())
        })
    });
    leaves.dedup();
    leaves
}

fn origin_precedes(
    ancestor: &AllOfOrigin,
    descendant: &AllOfOrigin,
    applicable: &[LocatedSchema<'_>],
) -> bool {
    applicable.iter().any(|located| {
        let ancestor_index = located
            .all_of_memberships
            .iter()
            .position(|membership| &membership.origin == ancestor);
        let descendant_index = located
            .all_of_memberships
            .iter()
            .position(|membership| &membership.origin == descendant);
        matches!((ancestor_index, descendant_index), (Some(left), Some(right)) if left < right)
    })
}

fn all_of_unsupported_finding(origin: AllOfOrigin, reason: &'static str) -> UnsupportedFinding {
    UnsupportedFinding {
        code: "applicator.all-of.ambiguous",
        keyword_location: origin.keyword_location,
        resource: origin.resource,
        parameters: serde_json::json!({
            "branchCount": origin.branch_count,
            "reason": reason,
        }),
    }
}

fn unsupported_region(
    binding: PointerBuf,
    parent_binding: Option<PointerBuf>,
    schema_locations: Vec<SchemaLocationDefinition>,
    presentation: NodePresentation,
    required: bool,
    findings: Vec<UnsupportedFinding>,
) -> UnsupportedRegion {
    UnsupportedRegion {
        binding,
        parent_binding,
        schema_locations,
        presentation,
        required,
        findings,
    }
}

fn infer_kind(applicable: &[LocatedSchema<'_>]) -> Option<ProjectedKind> {
    let mut declarations = applicable
        .iter()
        .filter_map(|located| located.schema.get("type"));
    let mut kinds = declared_projected_kinds(declarations.next()?)?;
    for declared in declarations {
        let declared = declared_projected_kinds(declared)?;
        kinds.retain(|kind| declared.contains(kind));
    }
    let accepts_null = kinds.remove(&ProjectedKind::Null);
    if kinds.is_empty() {
        return accepts_null.then_some(ProjectedKind::Null);
    }
    if accepts_null
        && kinds
            .iter()
            .any(|kind| matches!(kind, ProjectedKind::Object | ProjectedKind::Array))
    {
        return None;
    }
    if kinds == BTreeSet::from([ProjectedKind::Number, ProjectedKind::Integer]) {
        return Some(ProjectedKind::Number);
    }
    (kinds.len() == 1).then(|| *kinds.first().expect("one projected kind remains"))
}

fn declared_projected_kinds(declared: &Value) -> Option<BTreeSet<ProjectedKind>> {
    if let Some(kind) = declared.as_str() {
        let mut kinds = BTreeSet::new();
        insert_projected_kind(&mut kinds, projected_kind(kind)?);
        return Some(kinds);
    }
    let mut kinds = BTreeSet::new();
    for kind in declared.as_array()? {
        insert_projected_kind(&mut kinds, projected_kind(kind.as_str()?)?);
    }
    Some(kinds)
}

fn insert_projected_kind(kinds: &mut BTreeSet<ProjectedKind>, kind: ProjectedKind) {
    kinds.insert(kind);
    if kind == ProjectedKind::Number {
        kinds.insert(ProjectedKind::Integer);
    }
}

fn applicable_accepts_null(applicable: &[LocatedSchema<'_>]) -> bool {
    let declarations = applicable
        .iter()
        .filter_map(|located| located.schema.get("type"))
        .collect::<Vec<_>>();
    !declarations.is_empty()
        && declarations
            .iter()
            .all(|declaration| value_matches_type_declaration(&Value::Null, declaration))
}

fn projected_kind(kind: &str) -> Option<ProjectedKind> {
    match kind {
        "string" => Some(ProjectedKind::String),
        "number" => Some(ProjectedKind::Number),
        "integer" => Some(ProjectedKind::Integer),
        "boolean" => Some(ProjectedKind::Boolean),
        "null" => Some(ProjectedKind::Null),
        "object" => Some(ProjectedKind::Object),
        "array" => Some(ProjectedKind::Array),
        _ => None,
    }
}

struct ScalarChoices {
    values: Vec<Value>,
    selectable: bool,
    null_only: bool,
}

fn scalar_choices(
    applicable: &[LocatedSchema<'_>],
    independent_kind: Option<ProjectedKind>,
) -> Result<Option<ScalarChoices>, Vec<UnsupportedFinding>> {
    let enumerations = applicable
        .iter()
        .filter_map(|located| {
            located
                .schema
                .get("enum")
                .and_then(Value::as_array)
                .map(|values| (located, values))
        })
        .collect::<Vec<_>>();
    let constants = applicable
        .iter()
        .filter_map(|located| located.schema.get("const").map(|value| (located, value)))
        .collect::<Vec<_>>();
    let null_only = enumerations.is_empty()
        && constants.is_empty()
        && independent_kind == Some(ProjectedKind::Null);
    if enumerations.is_empty() && constants.is_empty() && !null_only {
        return Ok(None);
    }

    if let Some((located, _)) = constants
        .iter()
        .find(|(_, value)| value.is_array() || value.is_object())
    {
        return Err(vec![UnsupportedFinding {
            code: "validation.const.structured",
            keyword_location: append_pointer(Some(&located.pointer), ["const"]),
            resource: located.resource.clone(),
            parameters: serde_json::json!({}),
        }]);
    }

    if let Some((_, first)) = constants.first().copied()
        && let Some((located, _)) = constants
            .iter()
            .copied()
            .find(|(_, value)| !json_values_equal(first, value))
    {
        return Err(vec![UnsupportedFinding {
            code: "validation.const.conflicting",
            keyword_location: append_pointer(Some(&located.pointer), ["const"]),
            resource: located.resource.clone(),
            parameters: serde_json::json!({}),
        }]);
    }

    let mut values = if let Some((_, first)) = enumerations.first() {
        first.to_vec()
    } else if let Some((_, value)) = constants.first() {
        vec![(*value).clone()]
    } else {
        vec![Value::Null]
    };
    for (_, choices) in enumerations.iter().skip(1) {
        values.retain(|candidate| {
            choices
                .iter()
                .any(|choice| json_values_equal(candidate, choice))
        });
    }
    for (_, constant) in &constants {
        values.retain(|candidate| json_values_equal(candidate, constant));
    }
    values.retain(|candidate| {
        applicable.iter().all(|located| {
            located.schema.as_bool() != Some(false)
                && located
                    .schema
                    .get("type")
                    .is_none_or(|declared| value_matches_type_declaration(candidate, declared))
        })
    });

    let structured = values
        .iter()
        .any(|value| value.is_array() || value.is_object());
    if structured {
        let (located, keyword, code) = if let Some((located, _)) = enumerations.first() {
            (*located, "enum", "validation.enum.structured")
        } else {
            let (located, _) = constants
                .first()
                .expect("structured choices come from enum or const");
            (*located, "const", "validation.const.structured")
        };
        return Err(vec![UnsupportedFinding {
            code,
            keyword_location: append_pointer(Some(&located.pointer), [keyword]),
            resource: located.resource.clone(),
            parameters: serde_json::json!({}),
        }]);
    }

    let mut unique = Vec::with_capacity(values.len());
    for value in values {
        if !unique
            .iter()
            .any(|existing| json_values_equal(existing, &value))
        {
            unique.push(value);
        }
    }
    unique.sort_by_key(scalar_choice_sort_key);
    if unique.is_empty() {
        let (located, keyword, code) = if let Some((located, _)) = enumerations.first() {
            (*located, "enum", "validation.enum.incompatible")
        } else {
            let (located, _) = constants
                .first()
                .expect("an empty non-enum choice comes from const");
            (*located, "const", "validation.const.incompatible")
        };
        return Err(vec![UnsupportedFinding {
            code,
            keyword_location: append_pointer(Some(&located.pointer), [keyword]),
            resource: located.resource.clone(),
            parameters: serde_json::json!({}),
        }]);
    }

    Ok(Some(ScalarChoices {
        values: unique,
        selectable: !enumerations.is_empty() && constants.is_empty(),
        null_only,
    }))
}

fn value_matches_type_declaration(value: &Value, declaration: &Value) -> bool {
    if let Some(kind) = declaration.as_str() {
        return value_matches_type(value, kind);
    }
    declaration.as_array().is_some_and(|kinds| {
        kinds
            .iter()
            .filter_map(Value::as_str)
            .any(|kind| value_matches_type(value, kind))
    })
}

fn value_matches_type(value: &Value, kind: &str) -> bool {
    match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value
            .as_number()
            .is_some_and(|number| number_is_integer(&number.to_string())),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        _ => false,
    }
}

fn scalar_choice_sort_key(value: &Value) -> (u8, String) {
    match value {
        Value::Null => (0, String::new()),
        Value::Bool(value) => (1, value.to_string()),
        Value::Number(value) => (2, value.to_string()),
        Value::String(value) => (3, value.clone()),
        Value::Array(_) | Value::Object(_) => unreachable!("structured choices are rejected"),
    }
}

fn schema_locations(applicable: &[LocatedSchema<'_>]) -> Vec<SchemaLocationDefinition> {
    applicable
        .iter()
        .map(|located| (located.resource.clone(), located.pointer.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|(resource, pointer)| SchemaLocationDefinition { resource, pointer })
        .collect()
}

fn append_pointer<'a>(
    parent: Option<&PointerBuf>,
    tokens: impl IntoIterator<Item = &'a str>,
) -> PointerBuf {
    let mut pointer = parent.map_or_else(String::new, ToString::to_string);
    for token in tokens {
        pointer.push('/');
        pointer.push_str(&token.replace('~', "~0").replace('/', "~1"));
    }
    PointerBuf::parse(pointer).expect("escaped tokens form a valid JSON Pointer")
}

#[derive(Clone)]
struct SchemaLocationDefinition {
    resource: String,
    pointer: PointerBuf,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataSchemaAnnotations {
    formats: Vec<String>,
    defaults: Vec<Value>,
    deprecated: bool,
    read_only: bool,
    write_only: bool,
    examples: Vec<Value>,
    content_encodings: Vec<String>,
    content_media_types: Vec<String>,
    content_schemas: Vec<Value>,
}

impl DataSchemaAnnotations {
    pub fn formats(&self) -> impl Iterator<Item = &str> {
        self.formats.iter().map(String::as_str)
    }

    pub fn defaults(&self) -> impl Iterator<Item = &Value> {
        self.defaults.iter()
    }

    pub fn is_deprecated(&self) -> bool {
        self.deprecated
    }

    pub fn is_read_only(&self) -> bool {
        self.read_only
    }

    pub fn is_write_only(&self) -> bool {
        self.write_only
    }

    pub fn examples(&self) -> impl Iterator<Item = &Value> {
        self.examples.iter()
    }

    pub fn content_encodings(&self) -> impl Iterator<Item = &str> {
        self.content_encodings.iter().map(String::as_str)
    }

    pub fn content_media_types(&self) -> impl Iterator<Item = &str> {
        self.content_media_types.iter().map(String::as_str)
    }

    pub fn content_schemas(&self) -> impl Iterator<Item = &Value> {
        self.content_schemas.iter()
    }
}

#[derive(Clone)]
struct NodePresentation {
    label: String,
    help: Option<String>,
    annotations: DataSchemaAnnotations,
}

#[derive(Clone)]
struct ObjectDefinition {
    binding: PointerBuf,
    parent_binding: Option<PointerBuf>,
    schema_locations: Vec<SchemaLocationDefinition>,
    presentation: NodePresentation,
    required: bool,
    creation_seed: Value,
}

#[derive(Clone)]
struct ArrayDefinition {
    binding: PointerBuf,
    parent_binding: Option<PointerBuf>,
    schema_locations: Vec<SchemaLocationDefinition>,
    presentation: NodePresentation,
    required: bool,
    creation_seed: Value,
    min_items: Option<usize>,
    max_items: Option<usize>,
    item_template: ArrayItemTemplate,
}

#[derive(Clone)]
struct ArrayItemTemplate {
    controls: Vec<ControlDefinition>,
    objects: Vec<ObjectDefinition>,
    creation_seed: Value,
}

#[derive(Clone, Copy)]
pub struct ArrayDefinitionView<'a> {
    array: &'a ArrayDefinition,
}

impl<'a> ArrayDefinitionView<'a> {
    pub fn binding(&self) -> &'a str {
        self.array.binding.as_str()
    }

    pub fn parent_binding(&self) -> Option<&'a str> {
        self.array
            .parent_binding
            .as_ref()
            .map(|binding| binding.as_str())
    }

    pub fn schema_locations(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.array
            .schema_locations
            .iter()
            .map(|location| (location.resource.as_str(), location.pointer.as_str()))
    }

    pub fn label(&self) -> &'a str {
        &self.array.presentation.label
    }

    pub fn help(&self) -> Option<&'a str> {
        self.array.presentation.help.as_deref()
    }

    pub fn data_schema_annotations(&self) -> &'a DataSchemaAnnotations {
        &self.array.presentation.annotations
    }

    pub fn is_required(&self) -> bool {
        self.array.required
    }

    pub fn creation_seed(&self) -> &'a Value {
        &self.array.creation_seed
    }

    pub fn min_items(&self) -> Option<usize> {
        self.array.min_items
    }

    pub fn max_items(&self) -> Option<usize> {
        self.array.max_items
    }

    pub fn item_controls(&self) -> impl Iterator<Item = ControlDefinitionView<'a>> {
        self.array
            .item_template
            .controls
            .iter()
            .map(|control| ControlDefinitionView { control })
    }

    pub fn item_objects(&self) -> impl Iterator<Item = ObjectDefinitionView<'a>> {
        self.array
            .item_template
            .objects
            .iter()
            .map(|object| ObjectDefinitionView { object })
    }
}

#[derive(Clone, Copy)]
pub struct ObjectDefinitionView<'a> {
    object: &'a ObjectDefinition,
}

impl<'a> ObjectDefinitionView<'a> {
    pub fn binding(&self) -> &'a str {
        self.object.binding.as_str()
    }

    pub fn parent_binding(&self) -> Option<&'a str> {
        self.object
            .parent_binding
            .as_ref()
            .map(|binding| binding.as_str())
    }

    pub fn schema_locations(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.object
            .schema_locations
            .iter()
            .map(|location| (location.resource.as_str(), location.pointer.as_str()))
    }

    pub fn label(&self) -> &'a str {
        &self.object.presentation.label
    }

    pub fn help(&self) -> Option<&'a str> {
        self.object.presentation.help.as_deref()
    }

    pub fn data_schema_annotations(&self) -> &'a DataSchemaAnnotations {
        &self.object.presentation.annotations
    }

    pub fn creation_seed(&self) -> &'a Value {
        &self.object.creation_seed
    }

    pub fn is_required(&self) -> bool {
        self.object.required
    }
}

#[derive(Clone)]
struct UnsupportedRegion {
    binding: PointerBuf,
    parent_binding: Option<PointerBuf>,
    schema_locations: Vec<SchemaLocationDefinition>,
    presentation: NodePresentation,
    required: bool,
    findings: Vec<UnsupportedFinding>,
}

#[derive(Clone)]
struct UnsupportedFinding {
    code: &'static str,
    keyword_location: PointerBuf,
    resource: String,
    parameters: Value,
}

#[derive(Clone, Copy)]
pub struct UnsupportedRegionView<'a> {
    region: &'a UnsupportedRegion,
}

impl<'a> UnsupportedRegionView<'a> {
    pub fn binding(&self) -> &'a str {
        self.region.binding.as_str()
    }

    pub fn parent_binding(&self) -> Option<&'a str> {
        self.region
            .parent_binding
            .as_ref()
            .map(|binding| binding.as_str())
    }

    pub fn schema_locations(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.region
            .schema_locations
            .iter()
            .map(|location| (location.resource.as_str(), location.pointer.as_str()))
    }

    pub fn label(&self) -> &'a str {
        &self.region.presentation.label
    }

    pub fn help(&self) -> Option<&'a str> {
        self.region.presentation.help.as_deref()
    }

    pub fn data_schema_annotations(&self) -> &'a DataSchemaAnnotations {
        &self.region.presentation.annotations
    }

    pub fn is_required(&self) -> bool {
        self.region.required
    }
}

#[derive(Clone, Copy)]
pub struct ProjectionFindingView<'a> {
    finding: &'a UnsupportedFinding,
    binding: &'a PointerBuf,
    blocking: bool,
}

impl ProjectionFindingView<'_> {
    pub fn binding(&self) -> &str {
        self.binding.as_str()
    }

    pub fn code(&self) -> &'static str {
        self.finding.code
    }

    pub fn keyword_location(&self) -> &str {
        self.finding.keyword_location.as_str()
    }

    pub fn resource(&self) -> &str {
        &self.finding.resource
    }

    pub fn parameters(&self) -> &Value {
        &self.finding.parameters
    }

    pub fn is_blocking(&self) -> bool {
        self.blocking
    }
}

#[derive(Clone, Copy)]
pub struct ControlDefinitionView<'a> {
    control: &'a ControlDefinition,
}

impl<'a> ControlDefinitionView<'a> {
    pub fn binding(&self) -> &'a str {
        self.control.binding.as_str()
    }

    pub fn is_string(&self) -> bool {
        matches!(self.control.kind, ControlKind::String)
    }

    pub fn is_boolean(&self) -> bool {
        matches!(self.control.kind, ControlKind::Boolean)
    }

    pub fn is_number(&self) -> bool {
        matches!(self.control.kind, ControlKind::Number)
    }

    pub fn is_constant(&self) -> bool {
        matches!(self.control.kind, ControlKind::Constant)
    }

    pub fn is_choice(&self) -> bool {
        matches!(self.control.kind, ControlKind::Choice)
    }

    pub fn is_null(&self) -> bool {
        matches!(self.control.kind, ControlKind::Null)
    }

    pub fn choices(&self) -> impl Iterator<Item = &'a Value> {
        self.control.choices.iter()
    }

    pub fn accepts_null(&self) -> bool {
        self.control.accepts_null
    }

    pub fn parent_binding(&self) -> Option<&'a str> {
        self.control
            .parent_binding
            .as_ref()
            .map(|binding| binding.as_str())
    }

    pub fn schema_locations(&self) -> impl Iterator<Item = (&'a str, &'a str)> {
        self.control
            .schema_locations
            .iter()
            .map(|location| (location.resource.as_str(), location.pointer.as_str()))
    }

    pub fn label(&self) -> &'a str {
        &self.control.presentation.label
    }

    pub fn help(&self) -> Option<&'a str> {
        self.control.presentation.help.as_deref()
    }

    pub fn data_schema_annotations(&self) -> &'a DataSchemaAnnotations {
        &self.control.presentation.annotations
    }

    pub fn creation_seed(&self) -> Option<&'a Value> {
        self.control.creation_seed.as_ref()
    }

    pub fn is_required(&self) -> bool {
        self.control.required
    }
}

#[derive(Debug)]
pub enum CompileError {
    MissingProperties,
    UnsupportedReference(String),
    ResourceLimit(CompilationLimitError),
}

impl fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingProperties => {
                formatter.write_str("object data schema must declare properties")
            }
            Self::UnsupportedReference(reference) => {
                write!(formatter, "unsupported data-schema reference: {reference}")
            }
            Self::ResourceLimit(error) => error.fmt(formatter),
        }
    }
}

impl Error for CompileError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreateFormError {
    FormDataMustBeObject,
}

impl fmt::Display for CreateFormError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FormDataMustBeObject => formatter.write_str("form data must be an object"),
        }
    }
}

impl Error for CreateFormError {}

#[derive(Clone)]
pub struct Form {
    controls: Vec<ControlState>,
    arrays: Vec<ArrayState>,
    next_item_identity: u64,
    baseline_array_identities: Vec<Vec<u64>>,
    definition_fingerprint: DefinitionFingerprint,
    baseline: Value,
    form_data: Value,
    external_finding_batches: Vec<ExternalFindingBatch>,
    submission_attempted: bool,
    data_revision: u64,
    state_revision: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostArrayItem {
    Existing(u64),
    Fresh,
}

pub(crate) enum HostArrayChange {
    Preserve(Vec<HostArrayItem>),
    Replace,
}

pub(crate) struct HostItemWrite {
    pub(crate) array: usize,
    pub(crate) identity: u64,
    pub(crate) binding: PointerBuf,
}

#[derive(Clone)]
struct ArrayState {
    definition: ArrayDefinition,
    items: Vec<ArrayItemState>,
}

#[derive(Clone)]
struct ArrayItemState {
    identity: u64,
    controls: Vec<ControlState>,
}

impl ArrayItemState {
    fn new(identity: u64, definition: &ArrayDefinition) -> Self {
        Self {
            identity,
            controls: definition
                .item_template
                .controls
                .iter()
                .cloned()
                .map(ControlState::new)
                .collect(),
        }
    }
}

fn fresh_array_items(
    definition: &ArrayDefinition,
    count: usize,
    next_identity: &mut u64,
) -> Vec<ArrayItemState> {
    (0..count)
        .map(|_| {
            let identity = *next_identity;
            *next_identity += 1;
            ArrayItemState::new(identity, definition)
        })
        .collect()
}

fn fresh_array_items_for_data(
    definition: &ArrayDefinition,
    form_data: &Value,
    next_identity: &mut u64,
) -> Vec<ArrayItemState> {
    let count = definition
        .binding
        .resolve(form_data)
        .ok()
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    fresh_array_items(definition, count, next_identity)
}

fn array_items_with_identities(
    definition: &ArrayDefinition,
    identities: impl IntoIterator<Item = u64>,
) -> Vec<ArrayItemState> {
    identities
        .into_iter()
        .map(|identity| ArrayItemState::new(identity, definition))
        .collect()
}

#[derive(Clone, Copy)]
enum ControlLocation {
    Static(usize),
    ArrayItem {
        array: usize,
        item: usize,
        control: usize,
    },
}

impl Form {
    pub fn controls(&self) -> impl Iterator<Item = ControlView<'_>> {
        self.controls.iter().map(|control| ControlView {
            control,
            baseline: &self.baseline,
            form_data: &self.form_data,
            external_finding_batches: &self.external_finding_batches,
        })
    }

    pub fn control(&self, binding: &str) -> Option<ControlView<'_>> {
        let location = self.control_location(binding)?;
        Some(ControlView {
            control: self.control_state(location),
            baseline: &self.baseline,
            form_data: &self.form_data,
            external_finding_batches: &self.external_finding_batches,
        })
    }

    pub fn array_item_identities(&self, binding: &str) -> Option<Vec<u64>> {
        self.arrays
            .iter()
            .find(|array| array.definition.binding == binding)
            .map(|array| array.items.iter().map(|item| item.identity).collect())
    }

    pub fn array_item_binding(&self, binding: &str, identity: u64) -> Option<String> {
        let array = self
            .arrays
            .iter()
            .find(|array| array.definition.binding == binding)?;
        let index = array
            .items
            .iter()
            .position(|item| item.identity == identity)?;
        Some(array_item_pointer(&array.definition.binding, index).to_string())
    }

    pub fn array_can_append(&self, binding: &str) -> bool {
        let Some(array) = self
            .arrays
            .iter()
            .find(|array| array.definition.binding == binding)
        else {
            return false;
        };
        let Some(values) = array
            .definition
            .binding
            .resolve(&self.form_data)
            .ok()
            .and_then(Value::as_array)
        else {
            return false;
        };
        array
            .definition
            .max_items
            .is_none_or(|maximum| values.len() < maximum)
    }

    pub fn array_can_remove(&self, binding: &str) -> bool {
        let Some(array) = self
            .arrays
            .iter()
            .find(|array| array.definition.binding == binding)
        else {
            return false;
        };
        let Some(values) = array
            .definition
            .binding
            .resolve(&self.form_data)
            .ok()
            .and_then(Value::as_array)
        else {
            return false;
        };
        !values.is_empty()
            && array
                .definition
                .min_items
                .is_none_or(|minimum| values.len() > minimum)
    }

    pub fn array_can_move(&self, binding: &str) -> bool {
        self.arrays
            .iter()
            .find(|array| array.definition.binding == binding)
            .and_then(|array| {
                array
                    .definition
                    .binding
                    .resolve(&self.form_data)
                    .ok()
                    .and_then(Value::as_array)
            })
            .is_some_and(|values| values.len() > 1)
    }

    pub fn append_array_item(&mut self, binding: &str) -> Result<u64, EditError> {
        if !self.array_can_append(binding) {
            return Err(EditError::OperationNotAllowed(binding.to_owned()));
        }
        let array_index = self
            .arrays
            .iter()
            .position(|array| array.definition.binding == binding)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let seed = self.arrays[array_index]
            .definition
            .item_template
            .creation_seed
            .clone();
        self.arrays[array_index]
            .definition
            .binding
            .resolve_mut(&mut self.form_data)
            .ok()
            .and_then(Value::as_array_mut)
            .ok_or_else(|| EditError::UnresolvedControl(binding.to_owned()))?
            .push(seed);
        let identity = self.next_item_identity;
        self.next_item_identity += 1;
        let item = ArrayItemState::new(identity, &self.arrays[array_index].definition);
        self.arrays[array_index].items.push(item);
        self.external_finding_batches.clear();
        self.data_revision += 1;
        self.state_revision += 1;
        Ok(identity)
    }

    pub fn insert_array_item_before(
        &mut self,
        binding: &str,
        before: u64,
    ) -> Result<InsertedArrayItem, EditError> {
        if !self.array_can_append(binding) {
            return Err(EditError::OperationNotAllowed(binding.to_owned()));
        }
        let array_index = self
            .arrays
            .iter()
            .position(|array| array.definition.binding == binding)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let item_index = self.arrays[array_index]
            .items
            .iter()
            .position(|item| item.identity == before)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let seed = self.arrays[array_index]
            .definition
            .item_template
            .creation_seed
            .clone();
        self.arrays[array_index]
            .definition
            .binding
            .resolve_mut(&mut self.form_data)
            .ok()
            .and_then(Value::as_array_mut)
            .ok_or_else(|| EditError::UnresolvedControl(binding.to_owned()))?
            .insert(item_index, seed);
        let identity = self.next_item_identity;
        self.next_item_identity += 1;
        let item = ArrayItemState::new(identity, &self.arrays[array_index].definition);
        self.arrays[array_index].items.insert(item_index, item);
        let shifted = self.arrays[array_index].items[item_index + 1..]
            .iter()
            .map(|item| item.identity)
            .collect();
        self.external_finding_batches.clear();
        self.data_revision += 1;
        self.state_revision += 1;
        Ok(InsertedArrayItem { identity, shifted })
    }

    pub fn remove_array_item(
        &mut self,
        binding: &str,
        identity: u64,
    ) -> Result<RemovedArrayItem, EditError> {
        if !self.array_can_remove(binding) {
            return Err(EditError::OperationNotAllowed(binding.to_owned()));
        }
        let array_index = self
            .arrays
            .iter()
            .position(|array| array.definition.binding == binding)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let item_index = self.arrays[array_index]
            .items
            .iter()
            .position(|item| item.identity == identity)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let shifted = self.arrays[array_index].items[item_index + 1..]
            .iter()
            .map(|item| item.identity)
            .collect();
        self.arrays[array_index]
            .definition
            .binding
            .resolve_mut(&mut self.form_data)
            .ok()
            .and_then(Value::as_array_mut)
            .ok_or_else(|| EditError::UnresolvedControl(binding.to_owned()))?
            .remove(item_index);
        self.arrays[array_index].items.remove(item_index);
        self.external_finding_batches.clear();
        self.data_revision += 1;
        self.state_revision += 1;
        Ok(RemovedArrayItem { identity, shifted })
    }

    pub fn move_array_item_up(
        &mut self,
        binding: &str,
        identity: u64,
    ) -> Result<MovedArrayItem, EditError> {
        self.move_array_item(binding, identity, ArrayMove::Up)
    }

    pub fn move_array_item_down(
        &mut self,
        binding: &str,
        identity: u64,
    ) -> Result<MovedArrayItem, EditError> {
        self.move_array_item(binding, identity, ArrayMove::Down)
    }

    fn move_array_item(
        &mut self,
        binding: &str,
        identity: u64,
        direction: ArrayMove,
    ) -> Result<MovedArrayItem, EditError> {
        let array_index = self
            .arrays
            .iter()
            .position(|array| array.definition.binding == binding)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let item_index = self.arrays[array_index]
            .items
            .iter()
            .position(|item| item.identity == identity)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let destination = match direction {
            ArrayMove::Up => item_index.checked_sub(1),
            ArrayMove::Down => item_index
                .checked_add(1)
                .filter(|index| *index < self.arrays[array_index].items.len()),
        }
        .ok_or_else(|| EditError::OperationNotAllowed(binding.to_owned()))?;
        let values = self.arrays[array_index]
            .definition
            .binding
            .resolve_mut(&mut self.form_data)
            .ok()
            .and_then(Value::as_array_mut)
            .ok_or_else(|| EditError::UnresolvedControl(binding.to_owned()))?;
        let data_changed = !json_values_equal(&values[item_index], &values[destination]);
        values.swap(item_index, destination);
        let displaced = self.arrays[array_index].items[destination].identity;
        self.arrays[array_index].items.swap(item_index, destination);
        if data_changed {
            self.external_finding_batches.clear();
            self.data_revision += 1;
        }
        self.state_revision += 1;
        Ok(MovedArrayItem {
            identity,
            displaced,
            from: item_index,
            to: destination,
            data_changed,
        })
    }

    pub fn external_findings(&self) -> impl Iterator<Item = ExternalFindingView<'_>> {
        self.external_finding_batches.iter().flat_map(|batch| {
            batch
                .findings
                .iter()
                .map(move |finding| ExternalFindingView {
                    source: &batch.source,
                    finding,
                })
        })
    }

    pub fn form_data(&self) -> &Value {
        &self.form_data
    }

    pub fn control_accepts_value(&self, binding: &str, value: &Value) -> Option<bool> {
        self.control_location(binding).map(|location| {
            control_value_is_compatible(&self.control_state(location).definition, value)
        })
    }

    pub fn data_revision(&self) -> u64 {
        self.data_revision
    }

    pub fn state_revision(&self) -> u64 {
        self.state_revision
    }

    pub fn submission_attempted(&self) -> bool {
        self.submission_attempted
    }

    pub fn mark_state_changed(&mut self) {
        self.state_revision += 1;
    }

    pub fn edit_text(&mut self, binding: &str, input: &str) -> Result<(), EditError> {
        self.edit_text_with_integer_digit_limit(
            binding,
            input,
            DEFAULT_MAX_CANONICAL_INTEGER_DIGITS,
        )
    }

    pub(crate) fn edit_text_with_integer_digit_limit(
        &mut self,
        binding: &str,
        input: &str,
        max_canonical_integer_digits: usize,
    ) -> Result<(), EditError> {
        let (control_location, control_binding, next_parse_blocker, replacement) =
            self.prepare_text_edit(binding, input, max_canonical_integer_digits)?;
        let previous_edit_buffer = self.control_state(control_location).edit_buffer.clone();
        let previous_parse_blocker = self.control_state(control_location).parse_blocker;
        let form_data_changed = replacement.is_some();
        if let Some(value) = replacement {
            set_bound_value(&control_binding, &mut self.form_data, value)
                .map_err(|_| EditError::UnresolvedControl(binding.to_owned()))?;
        }

        let edit_state_changed = {
            let control = self.control_state_mut(control_location);
            control.edit_buffer = Some(input.to_owned());
            control.parse_blocker = next_parse_blocker;
            previous_edit_buffer != control.edit_buffer
                || previous_parse_blocker != control.parse_blocker
        };

        if form_data_changed {
            self.external_finding_batches.clear();
            self.data_revision += 1;
        }
        if form_data_changed || edit_state_changed {
            self.state_revision += 1;
        }

        Ok(())
    }

    pub(crate) fn prospective_text_form_data(
        &self,
        binding: &str,
        input: &str,
        max_canonical_integer_digits: usize,
    ) -> Result<Option<Value>, EditError> {
        let (_, control_binding, _, replacement) =
            self.prepare_text_edit(binding, input, max_canonical_integer_digits)?;
        let Some(value) = replacement else {
            return Ok(None);
        };
        let mut candidate = self.form_data.clone();
        set_bound_value(&control_binding, &mut candidate, value)
            .map_err(|_| EditError::UnresolvedControl(binding.to_owned()))?;
        Ok(Some(candidate))
    }

    pub(crate) fn prospective_edit_buffer_metrics(
        &self,
        binding: &str,
        input: &str,
    ) -> Result<(usize, usize), EditError> {
        let control_location = self
            .control_location(binding)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let controls = self.controls.iter().chain(
            self.arrays
                .iter()
                .flat_map(|array| array.items.iter())
                .flat_map(|item| item.controls.iter()),
        );
        let (mut active, mut bytes) = controls.fold((0usize, 0usize), |metrics, control| {
            control.edit_buffer.as_ref().map_or(metrics, |buffer| {
                (
                    metrics.0.saturating_add(1),
                    metrics.1.saturating_add(buffer.len()),
                )
            })
        });
        let previous = self.control_state(control_location).edit_buffer.as_deref();
        if previous.is_none() {
            active = active.saturating_add(1);
        }
        bytes = bytes
            .saturating_sub(previous.map_or(0, str::len))
            .saturating_add(input.len());
        Ok((active, bytes))
    }

    fn prepare_text_edit(
        &self,
        binding: &str,
        input: &str,
        max_canonical_integer_digits: usize,
    ) -> Result<
        (
            ControlLocation,
            PointerBuf,
            Option<ParseBlocker>,
            Option<Value>,
        ),
        EditError,
    > {
        let control_location = self
            .control_location(binding)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let definition = self.control_state(control_location).definition.clone();
        let kind = definition.kind;
        let control_binding = PointerBuf::parse(binding.to_owned())
            .map_err(|_| EditError::UnknownControl(binding.to_owned()))?;
        let parsed = match kind {
            ControlKind::String => Ok(Value::String(input.to_owned())),
            ControlKind::Number => parse_number(input),
            ControlKind::Integer => parse_integer_with_limit(input, max_canonical_integer_digits),
            ControlKind::Boolean
            | ControlKind::Choice
            | ControlKind::Constant
            | ControlKind::Null => {
                return Err(EditError::OperationNotAllowed(binding.to_owned()));
            }
        };

        let current = control_binding.resolve(&self.form_data).ok();
        if let Some(current) = current
            && !current.is_null()
            && !control_value_is_compatible(&definition, current)
        {
            return Err(EditError::OperationNotAllowed(binding.to_owned()));
        }
        if current.is_some_and(Value::is_null) && !definition.accepts_null {
            return Err(EditError::OperationNotAllowed(binding.to_owned()));
        }
        if current.is_none() && !binding_parent_is_object(&control_binding, &self.form_data) {
            return Err(EditError::UnresolvedControl(binding.to_owned()));
        }
        let (next_parse_blocker, replacement) = match parsed {
            Ok(value) => {
                let unchanged = match (kind, current) {
                    (ControlKind::String, Some(current)) => current == &value,
                    (ControlKind::Number | ControlKind::Integer, Some(current)) => {
                        numbers_equal(current, &value)
                    }
                    (ControlKind::String | ControlKind::Number | ControlKind::Integer, None) => {
                        false
                    }
                    (
                        ControlKind::Boolean
                        | ControlKind::Choice
                        | ControlKind::Constant
                        | ControlKind::Null,
                        _,
                    ) => unreachable!(),
                };
                (None, (!unchanged).then_some(value))
            }
            Err(blocker) => (Some(blocker), None),
        };

        Ok((
            control_location,
            control_binding,
            next_parse_blocker,
            replacement,
        ))
    }

    pub fn set_value(&mut self, binding: &str, value: &Value) -> Result<(), EditError> {
        let control_location = self
            .control_location(binding)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let control = self.control_state(control_location);
        if !control_value_is_compatible(&control.definition, value) {
            return Err(EditError::OperationNotAllowed(binding.to_owned()));
        }
        let value = control
            .definition
            .choices
            .iter()
            .find(|choice| json_values_equal(choice, value))
            .cloned()
            .unwrap_or_else(|| value.clone());
        let control_binding = PointerBuf::parse(binding.to_owned())
            .map_err(|_| EditError::UnknownControl(binding.to_owned()))?;
        if control_binding
            .resolve(&self.form_data)
            .is_ok_and(|current| json_values_equal(current, &value))
        {
            return Ok(());
        }
        set_bound_value(&control_binding, &mut self.form_data, value)
            .map_err(|_| EditError::UnresolvedControl(binding.to_owned()))?;
        self.control_state_mut(control_location).clear_edit_state();
        self.external_finding_batches.clear();
        self.data_revision += 1;
        self.state_revision += 1;
        Ok(())
    }

    pub fn materialize_structure(&mut self, binding: &str, seed: &Value) -> Result<(), EditError> {
        let binding = PointerBuf::parse(binding.to_owned())
            .map_err(|_| EditError::UnknownControl(binding.to_owned()))?;
        if binding.resolve(&self.form_data).is_ok() || !(seed.is_object() || seed.is_array()) {
            return Err(EditError::OperationNotAllowed(binding.to_string()));
        }
        set_bound_value(&binding, &mut self.form_data, seed.clone())
            .map_err(|_| EditError::UnresolvedControl(binding.to_string()))?;
        self.reconcile_authoritative_arrays(&binding);
        self.external_finding_batches.clear();
        self.data_revision += 1;
        self.state_revision += 1;
        Ok(())
    }

    pub fn replace_structure(&mut self, binding: &str, value: &Value) -> Result<(), EditError> {
        let binding = PointerBuf::parse(binding.to_owned())
            .map_err(|_| EditError::UnknownControl(binding.to_owned()))?;
        if !(value.is_object() || value.is_array())
            || !binding.resolve(&self.form_data).is_ok_and(|current| {
                value.is_object() && !current.is_object() || value.is_array() && !current.is_array()
            })
        {
            return Err(EditError::OperationNotAllowed(binding.to_string()));
        }
        set_bound_value(&binding, &mut self.form_data, value.clone())
            .map_err(|_| EditError::UnresolvedControl(binding.to_string()))?;
        self.reconcile_authoritative_arrays(&binding);
        for control in &mut self.controls {
            if control.definition.binding.starts_with(&binding) {
                control.clear_edit_state();
            }
        }
        for array in &mut self.arrays {
            for (index, item) in array.items.iter_mut().enumerate() {
                let item_binding = array_item_pointer(&array.definition.binding, index);
                for control in &mut item.controls {
                    let control_binding =
                        append_relative_pointer(&item_binding, &control.definition.binding);
                    if control_binding.starts_with(&binding) {
                        control.clear_edit_state();
                    }
                }
            }
        }
        self.external_finding_batches.clear();
        self.data_revision += 1;
        self.state_revision += 1;
        Ok(())
    }

    pub fn remove_value(&mut self, binding: &str) -> Result<(), EditError> {
        let binding = PointerBuf::parse(binding.to_owned())
            .map_err(|_| EditError::UnknownControl(binding.to_owned()))?;
        self.remove_bound_value(&binding)?;
        self.reconcile_authoritative_arrays(&binding);
        for control in &mut self.controls {
            if control.definition.binding.starts_with(&binding) {
                control.clear_edit_state();
            }
        }
        for array in &mut self.arrays {
            for (index, item) in array.items.iter_mut().enumerate() {
                let item_binding = array_item_pointer(&array.definition.binding, index);
                for control in &mut item.controls {
                    let control_binding =
                        append_relative_pointer(&item_binding, &control.definition.binding);
                    if control_binding.starts_with(&binding) {
                        control.clear_edit_state();
                    }
                }
            }
        }
        Ok(())
    }

    fn reconcile_authoritative_arrays(&mut self, binding: &PointerBuf) {
        for array in &mut self.arrays {
            if array.definition.binding != *binding
                && !array.definition.binding.starts_with(binding)
            {
                continue;
            }
            array.items = fresh_array_items_for_data(
                &array.definition,
                &self.form_data,
                &mut self.next_item_identity,
            );
        }
    }

    fn remove_bound_value(&mut self, binding: &PointerBuf) -> Result<(), EditError> {
        take_bound_value(binding, &mut self.form_data)
            .ok_or_else(|| EditError::UnresolvedControl(binding.to_string()))?;
        self.external_finding_batches.clear();
        self.data_revision += 1;
        self.state_revision += 1;
        Ok(())
    }

    pub fn bound_value_is_dirty(&self, binding: &str) -> bool {
        let Ok(binding) = PointerBuf::parse(binding.to_owned()) else {
            return false;
        };
        match (
            binding.resolve(&self.form_data),
            binding.resolve(&self.baseline),
        ) {
            (Ok(current), Ok(baseline)) => !json_values_equal(current, baseline),
            (Err(_), Err(_)) => false,
            _ => true,
        }
    }

    pub fn apply_external_findings(
        &mut self,
        source: impl Into<String>,
        data_revision: u64,
        findings: Vec<ExternalFinding>,
    ) -> Result<(), ApplyExternalFindingsError> {
        if data_revision != self.data_revision {
            return Err(ApplyExternalFindingsError::StaleDataRevision {
                current: self.data_revision,
                supplied: data_revision,
            });
        }

        let source = source.into();
        let mut findings = findings;
        findings.sort_by(|left, right| {
            left.instance_pointer
                .as_str()
                .cmp(right.instance_pointer.as_str())
                .then_with(|| left.code.cmp(&right.code))
                .then_with(|| left.blocking.cmp(&right.blocking))
        });
        findings.dedup();
        let existing_index = self
            .external_finding_batches
            .iter()
            .position(|existing| existing.source == source);
        if findings.is_empty() {
            if let Some(index) = existing_index {
                self.external_finding_batches.remove(index);
                self.state_revision += 1;
            }
            return Ok(());
        }

        let batch = ExternalFindingBatch {
            source,
            data_revision,
            findings,
        };
        if let Some(index) = existing_index {
            if self.external_finding_batches[index] == batch {
                return Ok(());
            }
            self.external_finding_batches[index] = batch;
        } else {
            self.external_finding_batches.push(batch);
            self.external_finding_batches
                .sort_by(|left, right| left.source.cmp(&right.source));
        }
        self.state_revision += 1;

        Ok(())
    }

    pub fn reset(&mut self) {
        let data_changed = !json_values_equal(&self.form_data, &self.baseline);
        let topology_changed =
            self.arrays
                .iter()
                .zip(&self.baseline_array_identities)
                .any(|(array, baseline)| {
                    !array
                        .items
                        .iter()
                        .map(|item| item.identity)
                        .eq(baseline.iter().copied())
                });
        let mut state_changed = data_changed || topology_changed || self.submission_attempted;
        for control in &mut self.controls {
            state_changed |= control.clear_lifecycle_state();
        }
        for array in &mut self.arrays {
            for item in &mut array.items {
                for control in &mut item.controls {
                    state_changed |= control.clear_lifecycle_state();
                }
            }
        }

        if data_changed {
            self.form_data = self.baseline.clone();
            self.data_revision += 1;
            self.external_finding_batches.clear();
        }
        if topology_changed {
            for (array, baseline_identities) in
                self.arrays.iter_mut().zip(&self.baseline_array_identities)
            {
                array.items = array_items_with_identities(
                    &array.definition,
                    baseline_identities.iter().copied(),
                );
            }
        }
        self.submission_attempted = false;
        if state_changed {
            self.state_revision += 1;
        }
    }

    pub fn reinitialize(&mut self, form_data: Value) -> Result<(), CreateFormError> {
        if !form_data.is_object() {
            return Err(CreateFormError::FormDataMustBeObject);
        }

        let data_changed = !json_values_equal(&self.form_data, &form_data);
        for control in &mut self.controls {
            control.clear_lifecycle_state();
        }
        for array in &mut self.arrays {
            array.items.clear();
            array.items = fresh_array_items_for_data(
                &array.definition,
                &form_data,
                &mut self.next_item_identity,
            );
        }
        self.baseline_array_identities = self
            .arrays
            .iter()
            .map(|array| array.items.iter().map(|item| item.identity).collect())
            .collect();

        if data_changed {
            self.form_data = form_data.clone();
        }
        self.baseline = form_data;
        self.external_finding_batches.clear();
        self.submission_attempted = false;
        self.data_revision += 1;
        self.state_revision += 1;

        Ok(())
    }

    pub fn replace_form_data(&mut self, form_data: Value) -> Result<(), CreateFormError> {
        let array_topologies = self
            .arrays
            .iter()
            .map(|array| {
                array
                    .items
                    .iter()
                    .map(|item| HostArrayItem::Existing(item.identity))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let array_changes = self
            .arrays
            .iter()
            .zip(array_topologies)
            .map(|(array, topology)| {
                if match (
                    array.definition.binding.resolve(&self.form_data),
                    array.definition.binding.resolve(&form_data),
                ) {
                    (Ok(current), Ok(candidate)) => !json_values_equal(current, candidate),
                    (Err(_), Err(_)) => false,
                    _ => true,
                } {
                    HostArrayChange::Replace
                } else {
                    HostArrayChange::Preserve(topology.clone())
                }
            })
            .collect::<Vec<_>>();
        self.apply_host_transaction(form_data, &[PointerBuf::new()], &array_changes, &[])
    }

    pub fn apply_host_transaction(
        &mut self,
        form_data: Value,
        writes: &[PointerBuf],
        array_changes: &[HostArrayChange],
        item_writes: &[HostItemWrite],
    ) -> Result<(), CreateFormError> {
        if !form_data.is_object() || array_changes.len() != self.arrays.len() {
            return Err(CreateFormError::FormDataMustBeObject);
        }

        let data_changed = !json_values_equal(&self.form_data, &form_data);
        for (array, change) in self.arrays.iter().zip(array_changes) {
            let HostArrayChange::Preserve(topology) = change else {
                continue;
            };
            let count = array
                .definition
                .binding
                .resolve(&form_data)
                .ok()
                .and_then(Value::as_array)
                .map_or(0, Vec::len);
            if topology.len() != count {
                return Err(CreateFormError::FormDataMustBeObject);
            }
            let mut seen = HashSet::new();
            if topology.iter().any(|item| match item {
                HostArrayItem::Existing(identity) => {
                    !seen.insert(*identity)
                        || !array.items.iter().any(|item| item.identity == *identity)
                }
                HostArrayItem::Fresh => false,
            }) {
                return Err(CreateFormError::FormDataMustBeObject);
            }
        }
        let topology_changed = self
            .arrays
            .iter()
            .zip(array_changes)
            .any(|(array, change)| match change {
                HostArrayChange::Replace => true,
                HostArrayChange::Preserve(topology) => !array
                    .items
                    .iter()
                    .map(|item| HostArrayItem::Existing(item.identity))
                    .eq(topology.iter().copied()),
            });
        let mut state_changed = data_changed || topology_changed;
        for control in &mut self.controls {
            if writes.iter().any(|write| {
                write.starts_with(&control.definition.binding)
                    || control.definition.binding.starts_with(write)
            }) {
                state_changed |= control.clear_edit_state();
            }
        }
        for write in item_writes {
            let Some(array) = self.arrays.get_mut(write.array) else {
                return Err(CreateFormError::FormDataMustBeObject);
            };
            let Some(item) = array
                .items
                .iter_mut()
                .find(|item| item.identity == write.identity)
            else {
                return Err(CreateFormError::FormDataMustBeObject);
            };
            for control in &mut item.controls {
                if write.binding.starts_with(&control.definition.binding)
                    || control.definition.binding.starts_with(&write.binding)
                {
                    state_changed |= control.clear_edit_state();
                }
            }
        }

        if data_changed {
            self.form_data = form_data;
            self.external_finding_batches.clear();
            self.data_revision += 1;
        }
        for (array, change) in self.arrays.iter_mut().zip(array_changes) {
            let topology = match change {
                HostArrayChange::Replace => {
                    array.items = fresh_array_items_for_data(
                        &array.definition,
                        &self.form_data,
                        &mut self.next_item_identity,
                    );
                    continue;
                }
                HostArrayChange::Preserve(topology) => topology,
            };
            if array
                .items
                .iter()
                .map(|item| HostArrayItem::Existing(item.identity))
                .eq(topology.iter().copied())
            {
                continue;
            }
            let mut previous = std::mem::take(&mut array.items);
            array.items = topology
                .iter()
                .map(|item| match item {
                    HostArrayItem::Existing(identity) => {
                        let index = previous
                            .iter()
                            .position(|item| item.identity == *identity)
                            .expect("host array topologies are validated before mutation");
                        previous.remove(index)
                    }
                    HostArrayItem::Fresh => {
                        let identity = self.next_item_identity;
                        self.next_item_identity += 1;
                        ArrayItemState::new(identity, &array.definition)
                    }
                })
                .collect();
        }
        if state_changed {
            self.state_revision += 1;
        }

        Ok(())
    }

    pub fn blur(&mut self, binding: &str) -> Result<(), EditError> {
        let location = self
            .control_location(binding)
            .ok_or_else(|| EditError::UnknownControl(binding.to_owned()))?;
        let control = self.control_state_mut(location);

        let state_changed = control.finalize_edit_buffer() | !control.touched;
        control.touched = true;
        if state_changed {
            self.state_revision += 1;
        }

        Ok(())
    }

    pub fn prepare_submission(&mut self) -> Result<SubmissionSnapshot, SubmissionFailure> {
        let mut blockers = self
            .controls
            .iter()
            .filter_map(|control| {
                control
                    .parse_blocker
                    .map(|reason| SubmissionBlocker::Parse {
                        binding: control.definition.binding.clone(),
                        reason,
                    })
            })
            .collect::<Vec<_>>();
        blockers.extend(self.arrays.iter().flat_map(|array| {
            array.items.iter().enumerate().flat_map(|(index, item)| {
                let item_binding = array_item_pointer(&array.definition.binding, index);
                item.controls.iter().filter_map(move |control| {
                    control
                        .parse_blocker
                        .map(|reason| SubmissionBlocker::Parse {
                            binding: append_relative_pointer(
                                &item_binding,
                                &control.definition.binding,
                            ),
                            reason,
                        })
                })
            })
        }));
        blockers.extend(self.external_finding_batches.iter().flat_map(|batch| {
            batch
                .findings
                .iter()
                .filter(|finding| finding.blocking)
                .map(|finding| SubmissionBlocker::External {
                    source: batch.source.clone(),
                    instance_pointer: finding.instance_pointer.clone(),
                    code: finding.code.clone(),
                })
        }));

        let mut state_changed = !self.submission_attempted;
        self.submission_attempted = true;
        for control in &mut self.controls {
            if control.finalize_edit_buffer() {
                state_changed = true;
            }
        }
        for array in &mut self.arrays {
            for item in &mut array.items {
                for control in &mut item.controls {
                    if control.finalize_edit_buffer() {
                        state_changed = true;
                    }
                }
            }
        }
        if state_changed {
            self.state_revision += 1;
        }

        if blockers.is_empty() {
            Ok(SubmissionSnapshot {
                form_data: self.form_data.clone(),
                data_revision: self.data_revision,
                definition_fingerprint: self.definition_fingerprint,
            })
        } else {
            Err(SubmissionFailure { blockers })
        }
    }

    fn control_location(&self, binding: &str) -> Option<ControlLocation> {
        if let Some(index) = self
            .controls
            .iter()
            .position(|control| control.definition.binding == binding)
        {
            return Some(ControlLocation::Static(index));
        }
        self.arrays
            .iter()
            .enumerate()
            .find_map(|(array_index, array)| {
                array
                    .items
                    .iter()
                    .enumerate()
                    .find_map(|(item_index, item)| {
                        let item_binding =
                            array_item_pointer(&array.definition.binding, item_index);
                        item.controls
                            .iter()
                            .enumerate()
                            .find_map(|(control_index, control)| {
                                (append_relative_pointer(
                                    &item_binding,
                                    &control.definition.binding,
                                )
                                .as_str()
                                    == binding)
                                    .then_some(ControlLocation::ArrayItem {
                                        array: array_index,
                                        item: item_index,
                                        control: control_index,
                                    })
                            })
                    })
            })
    }

    fn control_state(&self, location: ControlLocation) -> &ControlState {
        match location {
            ControlLocation::Static(index) => &self.controls[index],
            ControlLocation::ArrayItem {
                array,
                item,
                control,
            } => &self.arrays[array].items[item].controls[control],
        }
    }

    fn control_state_mut(&mut self, location: ControlLocation) -> &mut ControlState {
        match location {
            ControlLocation::Static(index) => &mut self.controls[index],
            ControlLocation::ArrayItem {
                array,
                item,
                control,
            } => &mut self.arrays[array].items[item].controls[control],
        }
    }
}

pub struct RemovedArrayItem {
    pub identity: u64,
    pub shifted: Vec<u64>,
}

pub struct InsertedArrayItem {
    pub identity: u64,
    pub shifted: Vec<u64>,
}

pub struct MovedArrayItem {
    pub identity: u64,
    pub displaced: u64,
    pub from: usize,
    pub to: usize,
    pub data_changed: bool,
}

#[derive(Clone, Copy)]
enum ArrayMove {
    Up,
    Down,
}

#[derive(Debug)]
pub enum EditError {
    UnknownControl(String),
    UnresolvedControl(String),
    OperationNotAllowed(String),
}

impl fmt::Display for EditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownControl(binding) => {
                write!(formatter, "unknown control binding: {binding}")
            }
            Self::UnresolvedControl(binding) => {
                write!(
                    formatter,
                    "control binding is absent from form data: {binding}"
                )
            }
            Self::OperationNotAllowed(binding) => {
                write!(formatter, "operation is not allowed for control: {binding}")
            }
        }
    }
}

impl Error for EditError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFinding {
    instance_pointer: PointerBuf,
    code: String,
    blocking: bool,
}

impl ExternalFinding {
    pub fn blocking(
        instance_pointer: &str,
        code: impl Into<String>,
    ) -> Result<Self, ExternalFindingError> {
        Self::new(instance_pointer, code, true)
    }

    pub fn advisory(
        instance_pointer: &str,
        code: impl Into<String>,
    ) -> Result<Self, ExternalFindingError> {
        Self::new(instance_pointer, code, false)
    }

    fn new(
        instance_pointer: &str,
        code: impl Into<String>,
        blocking: bool,
    ) -> Result<Self, ExternalFindingError> {
        let instance_pointer = PointerBuf::parse(instance_pointer.to_owned()).map_err(|_| {
            ExternalFindingError::InvalidInstancePointer(instance_pointer.to_owned())
        })?;

        Ok(Self {
            instance_pointer,
            code: code.into(),
            blocking,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalFindingError {
    InvalidInstancePointer(String),
}

impl fmt::Display for ExternalFindingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInstancePointer(instance_pointer) => {
                write!(
                    formatter,
                    "invalid external finding instance pointer: {instance_pointer}"
                )
            }
        }
    }
}

impl Error for ExternalFindingError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyExternalFindingsError {
    StaleDataRevision { current: u64, supplied: u64 },
}

impl fmt::Display for ApplyExternalFindingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StaleDataRevision { current, supplied } => write!(
                formatter,
                "external findings target data revision {supplied}, but the current revision is {current}"
            ),
        }
    }
}

impl Error for ApplyExternalFindingsError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseBlocker {
    InvalidNumber,
    InvalidInteger,
    ResourceLimitExceeded,
}

pub struct ControlView<'a> {
    control: &'a ControlState,
    baseline: &'a Value,
    form_data: &'a Value,
    external_finding_batches: &'a [ExternalFindingBatch],
}

impl<'a> ControlView<'a> {
    pub fn binding(&self) -> &'a str {
        self.control.definition.binding.as_str()
    }

    pub fn edit_buffer(&self) -> Option<&'a str> {
        self.control.edit_buffer.as_deref()
    }

    pub fn parse_blocker(&self) -> Option<ParseBlocker> {
        self.control.parse_blocker
    }

    pub fn is_touched(&self) -> bool {
        self.control.touched
    }

    pub fn is_dirty(&self) -> bool {
        let binding = &self.control.definition.binding;
        match (
            binding.resolve(self.form_data),
            binding.resolve(self.baseline),
        ) {
            (Ok(current), Ok(baseline)) => match self.control.definition.kind {
                ControlKind::String => current != baseline,
                ControlKind::Number | ControlKind::Integer => !numbers_equal(current, baseline),
                ControlKind::Boolean
                | ControlKind::Choice
                | ControlKind::Constant
                | ControlKind::Null => !json_values_equal(current, baseline),
            },
            (Err(current), Err(baseline)) => current != baseline,
            _ => true,
        }
    }

    pub fn external_findings(&self) -> impl Iterator<Item = ExternalFindingView<'a>> + 'a {
        let binding = &self.control.definition.binding;
        self.external_finding_batches.iter().flat_map(move |batch| {
            batch.findings.iter().filter_map(move |finding| {
                (finding.instance_pointer == *binding).then_some(ExternalFindingView {
                    source: &batch.source,
                    finding,
                })
            })
        })
    }
}

pub struct ExternalFindingView<'a> {
    source: &'a str,
    finding: &'a ExternalFinding,
}

impl<'a> ExternalFindingView<'a> {
    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn code(&self) -> &'a str {
        &self.finding.code
    }

    pub fn instance_pointer(&self) -> &'a str {
        self.finding.instance_pointer.as_str()
    }

    pub fn is_blocking(&self) -> bool {
        self.finding.blocking
    }
}

#[derive(Debug)]
pub struct SubmissionSnapshot {
    form_data: Value,
    data_revision: u64,
    definition_fingerprint: DefinitionFingerprint,
}

impl SubmissionSnapshot {
    pub fn form_data(&self) -> &Value {
        &self.form_data
    }

    pub fn data_revision(&self) -> u64 {
        self.data_revision
    }

    pub fn definition_fingerprint(&self) -> DefinitionFingerprint {
        self.definition_fingerprint
    }
}

#[derive(Debug)]
pub struct SubmissionFailure {
    blockers: Vec<SubmissionBlocker>,
}

impl SubmissionFailure {
    pub fn parse_blockers(&self) -> impl Iterator<Item = ParseSubmissionBlocker<'_>> {
        self.blockers.iter().filter_map(|blocker| match blocker {
            SubmissionBlocker::Parse { binding, reason } => Some(ParseSubmissionBlocker {
                binding,
                reason: *reason,
            }),
            SubmissionBlocker::External { .. } => None,
        })
    }

    pub fn external_blockers(&self) -> impl Iterator<Item = ExternalSubmissionBlocker<'_>> {
        self.blockers.iter().filter_map(|blocker| match blocker {
            SubmissionBlocker::External {
                source,
                instance_pointer,
                code,
            } => Some(ExternalSubmissionBlocker {
                source,
                instance_pointer,
                code,
            }),
            SubmissionBlocker::Parse { .. } => None,
        })
    }

    pub fn has_parse_blocker(&self, binding: &str) -> bool {
        self.parse_blockers()
            .any(|blocker| blocker.binding() == binding)
    }

    pub fn has_external_blocker(&self, source: &str, instance_pointer: &str, code: &str) -> bool {
        self.external_blockers().any(|blocker| {
            blocker.source() == source
                && blocker.instance_pointer() == instance_pointer
                && blocker.code() == code
        })
    }
}

#[derive(Clone, Copy)]
pub struct ParseSubmissionBlocker<'a> {
    binding: &'a PointerBuf,
    reason: ParseBlocker,
}

impl<'a> ParseSubmissionBlocker<'a> {
    pub fn binding(&self) -> &'a str {
        self.binding.as_str()
    }

    pub fn reason(&self) -> ParseBlocker {
        self.reason
    }
}

#[derive(Clone, Copy)]
pub struct ExternalSubmissionBlocker<'a> {
    source: &'a str,
    instance_pointer: &'a PointerBuf,
    code: &'a str,
}

impl<'a> ExternalSubmissionBlocker<'a> {
    pub fn source(&self) -> &'a str {
        self.source
    }

    pub fn instance_pointer(&self) -> &'a str {
        self.instance_pointer.as_str()
    }

    pub fn code(&self) -> &'a str {
        self.code
    }
}

#[derive(Debug)]
enum SubmissionBlocker {
    Parse {
        binding: PointerBuf,
        reason: ParseBlocker,
    },
    External {
        source: String,
        instance_pointer: PointerBuf,
        code: String,
    },
}

#[derive(Clone, PartialEq, Eq)]
struct ExternalFindingBatch {
    source: String,
    data_revision: u64,
    findings: Vec<ExternalFinding>,
}

#[derive(Clone)]
struct ControlDefinition {
    binding: PointerBuf,
    parent_binding: Option<PointerBuf>,
    kind: ControlKind,
    choices: Vec<Value>,
    accepts_null: bool,
    schema_locations: Vec<SchemaLocationDefinition>,
    presentation: NodePresentation,
    creation_seed: Option<Value>,
    required: bool,
}

#[derive(Clone)]
struct ControlState {
    definition: ControlDefinition,
    edit_buffer: Option<String>,
    parse_blocker: Option<ParseBlocker>,
    touched: bool,
}

impl ControlState {
    fn new(definition: ControlDefinition) -> Self {
        Self {
            definition,
            edit_buffer: None,
            parse_blocker: None,
            touched: false,
        }
    }

    fn finalize_edit_buffer(&mut self) -> bool {
        self.parse_blocker.is_none() && self.edit_buffer.take().is_some()
    }

    fn clear_edit_state(&mut self) -> bool {
        let changed = self.edit_buffer.is_some() || self.parse_blocker.is_some();
        self.edit_buffer = None;
        self.parse_blocker = None;
        changed
    }

    fn clear_lifecycle_state(&mut self) -> bool {
        let changed = self.clear_edit_state() | self.touched;
        self.touched = false;
        changed
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ControlKind {
    String,
    Number,
    Integer,
    Boolean,
    Choice,
    Constant,
    Null,
}

fn control_value_is_compatible(control: &ControlDefinition, value: &Value) -> bool {
    if value.is_null() {
        return control.accepts_null;
    }
    match control.kind {
        ControlKind::String => value.is_string(),
        ControlKind::Number => value.is_number(),
        ControlKind::Integer => value
            .as_number()
            .is_some_and(|number| number_is_integer(&number.to_string())),
        ControlKind::Boolean => value.is_boolean(),
        ControlKind::Choice | ControlKind::Constant | ControlKind::Null => control
            .choices
            .iter()
            .any(|choice| json_values_equal(choice, value)),
    }
}

fn binding_parent_is_object(binding: &PointerBuf, form_data: &Value) -> bool {
    binding
        .split_back()
        .and_then(|(parent, _)| parent.resolve(form_data).ok())
        .is_some_and(Value::is_object)
}

fn array_item_pointer(array: &PointerBuf, index: usize) -> PointerBuf {
    append_pointer(Some(array), [index.to_string().as_str()])
}

fn set_bound_value(binding: &PointerBuf, form_data: &mut Value, value: Value) -> Result<(), ()> {
    if let Ok(current) = binding.resolve_mut(form_data) {
        *current = value;
        return Ok(());
    }
    let (parent, property) = binding.split_back().ok_or(())?;
    parent
        .resolve_mut(form_data)
        .ok()
        .and_then(Value::as_object_mut)
        .ok_or(())?
        .insert(property.decoded().into_owned(), value);
    Ok(())
}

fn take_bound_value(binding: &PointerBuf, form_data: &mut Value) -> Option<Value> {
    let (parent, property) = binding.split_back()?;
    parent
        .resolve_mut(form_data)
        .ok()
        .and_then(Value::as_object_mut)
        .and_then(|object| object.remove(property.decoded().as_ref()))
}

fn fingerprint_compiled_definition(
    controls: &[ControlDefinition],
    objects: &[ObjectDefinition],
    arrays: &[ArrayDefinition],
    graph: &ResourceGraph,
    used_resources: &BTreeSet<String>,
) -> DefinitionFingerprint {
    let mut hasher = Sha256::new();
    hasher.update(b"schemaform-compiled-definition-v15\0");
    hash_fingerprint_length(&mut hasher, controls.len());
    for control in controls {
        hash_fingerprint_bytes(&mut hasher, control.binding.as_str().as_bytes());
        hasher.update([match control.kind {
            ControlKind::String => 0,
            ControlKind::Integer => 1,
            ControlKind::Boolean => 2,
            ControlKind::Choice => 3,
            ControlKind::Constant => 4,
            ControlKind::Number => 5,
            ControlKind::Null => 6,
        }]);
        hash_fingerprint_bytes(&mut hasher, control.presentation.label.as_bytes());
        hash_optional_fingerprint_bytes(&mut hasher, control.presentation.help.as_deref());
        if let Some(seed) = &control.creation_seed {
            hasher.update([1]);
            hash_json_value(&mut hasher, seed);
        } else {
            hasher.update([0]);
        }
        hasher.update([control.required as u8]);
        hasher.update([control.accepts_null as u8]);
        hash_fingerprint_length(&mut hasher, control.choices.len());
        for choice in &control.choices {
            hash_json_value(&mut hasher, choice);
        }
    }
    hash_fingerprint_length(&mut hasher, objects.len());
    for object in objects {
        hash_fingerprint_bytes(&mut hasher, object.binding.as_str().as_bytes());
        hash_fingerprint_bytes(&mut hasher, object.presentation.label.as_bytes());
        hash_optional_fingerprint_bytes(&mut hasher, object.presentation.help.as_deref());
        hasher.update([object.required as u8]);
        hash_json_value(&mut hasher, &object.creation_seed);
    }
    hash_fingerprint_length(&mut hasher, arrays.len());
    for array in arrays {
        hash_fingerprint_bytes(&mut hasher, array.binding.as_str().as_bytes());
        hash_fingerprint_bytes(&mut hasher, array.presentation.label.as_bytes());
        hash_optional_fingerprint_bytes(&mut hasher, array.presentation.help.as_deref());
        hasher.update([array.required as u8]);
        hash_optional_usize(&mut hasher, array.min_items);
        hash_optional_usize(&mut hasher, array.max_items);
        hash_json_value(&mut hasher, &array.creation_seed);
        hash_json_value(&mut hasher, &array.item_template.creation_seed);
        hash_fingerprint_length(&mut hasher, array.item_template.objects.len());
        for object in &array.item_template.objects {
            hash_fingerprint_bytes(&mut hasher, object.binding.as_str().as_bytes());
            hash_fingerprint_bytes(&mut hasher, object.presentation.label.as_bytes());
            hash_optional_fingerprint_bytes(&mut hasher, object.presentation.help.as_deref());
            hasher.update([object.required as u8]);
            hash_json_value(&mut hasher, &object.creation_seed);
        }
        hash_fingerprint_length(&mut hasher, array.item_template.controls.len());
        for control in &array.item_template.controls {
            hash_fingerprint_bytes(&mut hasher, control.binding.as_str().as_bytes());
            hasher.update([match control.kind {
                ControlKind::String => 0,
                ControlKind::Integer => 1,
                ControlKind::Boolean => 2,
                ControlKind::Choice => 3,
                ControlKind::Constant => 4,
                ControlKind::Number => 5,
                ControlKind::Null => 6,
            }]);
            hash_fingerprint_bytes(&mut hasher, control.presentation.label.as_bytes());
            hash_optional_fingerprint_bytes(&mut hasher, control.presentation.help.as_deref());
            hasher.update([control.required as u8]);
            hasher.update([control.accepts_null as u8]);
            hash_fingerprint_length(&mut hasher, control.choices.len());
            for choice in &control.choices {
                hash_json_value(&mut hasher, choice);
            }
        }
    }
    hash_fingerprint_length(&mut hasher, used_resources.len());
    for resource in used_resources {
        hash_fingerprint_bytes(&mut hasher, resource.as_bytes());
        hash_schema_value(
            &mut hasher,
            graph
                .document_for_resource(resource)
                .expect("used resources remain in the prepared graph"),
        );
    }

    DefinitionFingerprint(hasher.finalize().into())
}

fn hash_optional_usize(hasher: &mut Sha256, value: Option<usize>) {
    if let Some(value) = value {
        hasher.update([1]);
        hasher.update((value as u64).to_be_bytes());
    } else {
        hasher.update([0]);
    }
}

fn hash_schema_value(hasher: &mut Sha256, value: &Value) {
    match value {
        Value::Null => hasher.update([0]),
        Value::Bool(value) => hasher.update([1, *value as u8]),
        Value::Number(_) | Value::String(_) | Value::Array(_) => hash_json_value(hasher, value),
        Value::Object(values) => {
            hasher.update([5]);
            let mut entries = values
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "$schema"
                            | "$comment"
                            | "default"
                            | "deprecated"
                            | "description"
                            | "examples"
                            | "readOnly"
                            | "writeOnly"
                    )
                })
                .collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(key, _)| *key);
            hash_fingerprint_length(hasher, entries.len());
            for (key, value) in entries {
                hash_fingerprint_bytes(hasher, key.as_bytes());
                match key.as_str() {
                    "$defs" | "definitions" | "dependentSchemas" | "patternProperties"
                    | "properties" => hash_schema_map(hasher, value),
                    "additionalItems"
                    | "additionalProperties"
                    | "contains"
                    | "contentSchema"
                    | "else"
                    | "if"
                    | "items"
                    | "not"
                    | "propertyNames"
                    | "then"
                    | "unevaluatedItems"
                    | "unevaluatedProperties" => hash_schema_value(hasher, value),
                    "allOf" | "anyOf" | "oneOf" | "prefixItems" => hash_schema_array(hasher, value),
                    "required"
                        if value
                            .as_array()
                            .is_some_and(|items| items.iter().all(Value::is_string)) =>
                    {
                        hash_unordered_strings(hasher, value)
                    }
                    "enum" if value.is_array() => hash_unordered_json_values(hasher, value),
                    _ => hash_json_value(hasher, value),
                }
            }
        }
    }
}

fn hash_schema_map(hasher: &mut Sha256, value: &Value) {
    let Some(values) = value.as_object() else {
        hash_json_value(hasher, value);
        return;
    };
    hasher.update([6]);
    let mut entries = values.iter().collect::<Vec<_>>();
    entries.sort_unstable_by_key(|(key, _)| *key);
    hash_fingerprint_length(hasher, entries.len());
    for (key, value) in entries {
        hash_fingerprint_bytes(hasher, key.as_bytes());
        hash_schema_value(hasher, value);
    }
}

fn hash_schema_array(hasher: &mut Sha256, value: &Value) {
    let Some(values) = value.as_array() else {
        hash_json_value(hasher, value);
        return;
    };
    hasher.update([7]);
    hash_fingerprint_length(hasher, values.len());
    for value in values {
        hash_schema_value(hasher, value);
    }
}

fn hash_unordered_strings(hasher: &mut Sha256, value: &Value) {
    let mut values = value
        .as_array()
        .expect("required was checked to be an array")
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    values.sort_unstable();
    hasher.update([8]);
    hash_fingerprint_length(hasher, values.len());
    for value in values {
        hash_fingerprint_bytes(hasher, value.as_bytes());
    }
}

fn hash_unordered_json_values(hasher: &mut Sha256, value: &Value) {
    let mut values = value
        .as_array()
        .expect("enum was checked to be an array")
        .iter()
        .map(|value| {
            let mut item_hasher = Sha256::new();
            hash_json_value(&mut item_hasher, value);
            <[u8; 32]>::from(item_hasher.finalize())
        })
        .collect::<Vec<_>>();
    values.sort_unstable();
    hasher.update([15]);
    hash_fingerprint_length(hasher, values.len());
    for value in values {
        hasher.update(value);
    }
}

fn hash_json_value(hasher: &mut Sha256, value: &Value) {
    match value {
        Value::Null => hasher.update([9]),
        Value::Bool(value) => hasher.update([10, *value as u8]),
        Value::Number(value) => {
            hasher.update([11]);
            if let Some(normal) = number_normal_form(&value.to_string()) {
                hasher.update([normal.negative as u8]);
                hash_fingerprint_bytes(hasher, normal.significant_digits.as_bytes());
                hash_fingerprint_bytes(hasher, normal.scale.to_string().as_bytes());
            } else {
                hash_fingerprint_bytes(hasher, value.to_string().as_bytes());
            }
        }
        Value::String(value) => {
            hasher.update([12]);
            hash_fingerprint_bytes(hasher, value.as_bytes());
        }
        Value::Array(values) => {
            hasher.update([13]);
            hash_fingerprint_length(hasher, values.len());
            for value in values {
                hash_json_value(hasher, value);
            }
        }
        Value::Object(values) => {
            hasher.update([14]);
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(key, _)| *key);
            hash_fingerprint_length(hasher, entries.len());
            for (key, value) in entries {
                hash_fingerprint_bytes(hasher, key.as_bytes());
                hash_json_value(hasher, value);
            }
        }
    }
}

pub(crate) fn semantic_json_fingerprint(value: &Value) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hash_json_value(&mut hasher, value);
    hasher.finalize().into()
}

fn hash_fingerprint_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hash_fingerprint_length(hasher, bytes.len());
    hasher.update(bytes);
}

fn hash_optional_fingerprint_bytes(hasher: &mut Sha256, value: Option<&str>) {
    if let Some(value) = value {
        hasher.update([1]);
        hash_fingerprint_bytes(hasher, value.as_bytes());
    } else {
        hasher.update([0]);
    }
}

fn hash_fingerprint_length(hasher: &mut Sha256, length: usize) {
    let length = u64::try_from(length).expect("JSON values cannot exceed the fingerprint length");
    hasher.update(length.to_be_bytes());
}

fn parse_integer(input: &str) -> Result<Value, ParseBlocker> {
    parse_integer_with_limit(input, DEFAULT_MAX_CANONICAL_INTEGER_DIGITS)
}

fn parse_integer_with_limit(input: &str, maximum_digits: usize) -> Result<Value, ParseBlocker> {
    if !matches!(serde_json::from_str(input), Ok(Value::Number(_))) {
        return Err(ParseBlocker::InvalidInteger);
    }

    let canonical = canonical_integer_with_limit(input.trim(), maximum_digits)?;
    serde_json::from_str(&canonical).map_err(|_| ParseBlocker::InvalidInteger)
}

fn parse_number(input: &str) -> Result<Value, ParseBlocker> {
    match serde_json::from_str(input) {
        Ok(value @ Value::Number(_)) => Ok(value),
        _ => Err(ParseBlocker::InvalidNumber),
    }
}

fn numbers_equal(left: &Value, right: &Value) -> bool {
    if left == right {
        return true;
    }

    match (left, right) {
        (Value::Number(left), Value::Number(right)) => match (
            number_normal_form(&left.to_string()),
            number_normal_form(&right.to_string()),
        ) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn json_values_equal(left: &Value, right: &Value) -> bool {
    if left == right {
        return true;
    }

    match (left, right) {
        (Value::Number(_), Value::Number(_)) => numbers_equal(left, right),
        (Value::Array(left), Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| json_values_equal(left, right))
        }
        (Value::Object(left), Value::Object(right)) => {
            left.len() == right.len()
                && left.iter().all(|(key, left)| {
                    right
                        .get(key)
                        .is_some_and(|right| json_values_equal(left, right))
                })
        }
        _ => false,
    }
}

pub(crate) fn preserve_semantically_equal_values(current: &Value, candidate: &mut Value) {
    if json_values_equal(current, candidate) {
        *candidate = current.clone();
        return;
    }

    match (current, candidate) {
        (Value::Array(current), Value::Array(candidate)) => {
            for (current, candidate) in current.iter().zip(candidate) {
                preserve_semantically_equal_values(current, candidate);
            }
        }
        (Value::Object(current), Value::Object(candidate)) => {
            for (key, current) in current {
                if let Some(candidate) = candidate.get_mut(key) {
                    preserve_semantically_equal_values(current, candidate);
                }
            }
        }
        _ => {}
    }
}

#[derive(PartialEq, Eq)]
struct NumberNormalForm {
    negative: bool,
    significant_digits: String,
    scale: BigInt,
}

fn number_normal_form(input: &str) -> Option<NumberNormalForm> {
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
        return Some(NumberNormalForm {
            negative: false,
            significant_digits: "0".to_owned(),
            scale: BigInt::from(0_u8),
        });
    }

    let digits = digits.trim_start_matches('0');
    let trailing_zeros = digits
        .bytes()
        .rev()
        .take_while(|digit| *digit == b'0')
        .count();
    let significant_digits = digits[..digits.len() - trailing_zeros].to_owned();
    let scale = exponent - BigInt::from(fraction.len()) + BigInt::from(trailing_zeros);
    Some(NumberNormalForm {
        negative,
        significant_digits,
        scale,
    })
}

fn number_is_integer(input: &str) -> bool {
    number_normal_form(input).is_some_and(|normal| normal.scale >= BigInt::from(0_u8))
}

fn canonical_integer_with_limit(
    input: &str,
    maximum_digits: usize,
) -> Result<String, ParseBlocker> {
    let (negative, unsigned) = match input.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => (false, input),
    };

    let (significand, exponent) = match unsigned.find(['e', 'E']) {
        Some(index) => (&unsigned[..index], &unsigned[index + 1..]),
        None => (unsigned, "0"),
    };
    let (whole, fraction) = significand.split_once('.').unwrap_or((significand, ""));

    let mut digits = String::with_capacity(whole.len() + fraction.len());
    digits.push_str(whole);
    digits.push_str(fraction);
    if digits.bytes().all(|digit| digit == b'0') {
        if maximum_digits == 0 {
            return Err(ParseBlocker::ResourceLimitExceeded);
        }
        return Ok("0".to_owned());
    }
    digits = digits.trim_start_matches('0').to_owned();

    let exponent = exponent.parse::<i64>().map_err(|_| {
        if exponent.starts_with('-') {
            ParseBlocker::InvalidInteger
        } else {
            ParseBlocker::ResourceLimitExceeded
        }
    })?;
    let fraction_len =
        i64::try_from(fraction.len()).map_err(|_| ParseBlocker::ResourceLimitExceeded)?;
    let shift = exponent
        .checked_sub(fraction_len)
        .ok_or(if exponent.is_negative() {
            ParseBlocker::InvalidInteger
        } else {
            ParseBlocker::ResourceLimitExceeded
        })?;

    if shift >= 0 {
        let added_digits =
            usize::try_from(shift).map_err(|_| ParseBlocker::ResourceLimitExceeded)?;
        let total_digits = digits
            .len()
            .checked_add(added_digits)
            .ok_or(ParseBlocker::ResourceLimitExceeded)?;
        if total_digits > maximum_digits {
            return Err(ParseBlocker::ResourceLimitExceeded);
        }
        digits.extend(std::iter::repeat_n('0', added_digits));
    } else {
        let removed_digits = shift.unsigned_abs();
        if removed_digits
            >= u64::try_from(digits.len()).map_err(|_| ParseBlocker::ResourceLimitExceeded)?
        {
            return Err(ParseBlocker::InvalidInteger);
        } else {
            let removed_digits =
                usize::try_from(removed_digits).map_err(|_| ParseBlocker::ResourceLimitExceeded)?;
            let split = digits.len() - removed_digits;
            if !digits[split..].bytes().all(|digit| digit == b'0') {
                return Err(ParseBlocker::InvalidInteger);
            }
            digits.truncate(split);
        }
    }

    if digits.len() > maximum_digits {
        return Err(ParseBlocker::ResourceLimitExceeded);
    }

    let canonical_digits = digits.trim_start_matches('0');
    if canonical_digits.is_empty() {
        return Ok("0".to_owned());
    }

    Ok(if negative {
        format!("-{canonical_digits}")
    } else {
        canonical_digits.to_owned()
    })
}

#[cfg(test)]
mod form_trace;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_equality_is_non_expanding_and_not_limited_to_machine_exponents() {
        let exponent = serde_json::from_str("1e4096").expect("the exponent form should parse");
        let plain = serde_json::from_str(&format!("1{}", "0".repeat(4096)))
            .expect("the plain form should parse");
        assert!(numbers_equal(&exponent, &plain));

        let huge_exponent = serde_json::from_str("1e9223372036854775808")
            .expect("the huge exponent form should parse");
        let equivalent = serde_json::from_str("10e9223372036854775807")
            .expect("the equivalent huge exponent form should parse");
        assert!(numbers_equal(&huge_exponent, &equivalent));

        let negative_zero =
            serde_json::from_str("-0e9223372036854775808").expect("negative zero should parse");
        let zero = serde_json::from_str("0").expect("zero should parse");
        assert!(numbers_equal(&negative_zero, &zero));

        let decimal = serde_json::from_str("1.20").expect("the decimal form should parse");
        let exponent = serde_json::from_str("120e-2").expect("the exponent form should parse");
        assert!(numbers_equal(&decimal, &exponent));
    }
}
