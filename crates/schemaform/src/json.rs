use std::{error::Error, fmt};

use serde_json::Value;

use crate::{CompilationProfile, JsonPointer, ResourceLimitError, ResourceLimitPhase, ui};

/// Parses and structurally bounds a data schema received as bytes.
///
/// Successful parsing does not make the schema safe for hostile evaluator work;
/// compilation accepts only application-trusted data schemas.
/// Syntax failures retain the parser's reason and source position in
/// [`JsonParseError::Syntax`].
pub fn parse_data_schema(
    bytes: &[u8],
    profile: &CompilationProfile,
) -> Result<Value, JsonParseError> {
    crate::limits::check_schema_source(bytes, profile).map_err(JsonParseError::CompilationLimit)?;
    let value = serde_json::from_slice(bytes)
        .map_err(|error| JsonParseError::Syntax(JsonSyntaxError::from_serde_json(error)))?;
    crate::limits::check_parsed_schema(bytes.len(), &value, profile)
        .map_err(JsonParseError::CompilationLimit)?;
    Ok(value)
}

/// Parses form data while enforcing source and post-parse structural limits.
///
/// Limit failures report their exact construction dimension, bound, observation,
/// and nearest input path in [`JsonParseError::ResourceLimit`].
pub fn parse_form_data(bytes: &[u8], limits: &FormDataLimits) -> Result<Value, JsonParseError> {
    crate::limits::check_input_source(bytes, limits.input_limits())
        .map_err(|error| JsonParseError::ResourceLimit(resource_limit(error)))?;
    let value = serde_json::from_slice(bytes)
        .map_err(|error| JsonParseError::Syntax(JsonSyntaxError::from_serde_json(error)))?;
    crate::limits::check_input_value(&value, limits.input_limits())
        .map_err(|error| JsonParseError::ResourceLimit(resource_limit(error)))?;
    Ok(value)
}

/// Parses the stable UI-schema v1 wire format.
///
/// Version 1 freezes accepted JSON documents and their framework-neutral, headless meaning.
/// Syntax and resource-limit failures retain structured parser and limit details.
pub fn parse_ui_schema_v1(
    bytes: &[u8],
    profile: &CompilationProfile,
) -> Result<ui::v1::UiSchema, JsonParseError> {
    crate::limits::check_input_source(bytes, profile.ui_schema_limits())
        .map_err(|error| JsonParseError::ResourceLimit(resource_limit(error)))?;
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let ui_schema: ui::v1::UiSchema =
        serde_path_to_error::deserialize(&mut deserializer).map_err(|error| {
            if error.inner().is_syntax() || error.inner().is_eof() {
                JsonParseError::Syntax(JsonSyntaxError::from_serde_json(error.into_inner()))
            } else {
                JsonParseError::InvalidUiSchema {
                    location: ui_schema_error_location(&error),
                    reason: error.inner().to_string(),
                }
            }
        })?;
    deserializer
        .end()
        .map_err(|error| JsonParseError::Syntax(JsonSyntaxError::from_serde_json(error)))?;
    ui_schema.validate_limits(profile).map_err(|error| {
        if let Some(limit) = error.limit {
            JsonParseError::ResourceLimit(resource_limit(limit))
        } else {
            JsonParseError::InvalidUiSchema {
                location: JsonPointer::parse(error.location)
                    .expect("UI-schema extension locations are valid JSON Pointers"),
                reason: error.kind.to_string(),
            }
        }
    })?;
    Ok(ui_schema)
}

fn resource_limit(error: crate::limits::InputLimitError) -> ResourceLimitError {
    ResourceLimitError::new(
        ResourceLimitPhase::Construction,
        error.dimension,
        error.maximum,
        error.observed,
        JsonPointer::parse(error.pointer).expect("input limit scans produce valid JSON Pointers"),
    )
}

fn ui_schema_error_location(error: &serde_path_to_error::Error<serde_json::Error>) -> JsonPointer {
    use serde_path_to_error::Segment;

    let tokens = error
        .path()
        .iter()
        .filter_map(|segment| match segment {
            Segment::Seq { index } => Some(index.to_string()),
            Segment::Map { key } => Some(key.clone()),
            Segment::Enum { .. } | Segment::Unknown => None,
        })
        .collect::<Vec<_>>();
    let pointer = tokens
        .into_iter()
        .fold(String::new(), |mut pointer, token| {
            pointer.push('/');
            pointer.push_str(&token.replace('~', "~0").replace('/', "~1"));
            pointer
        });
    JsonPointer::parse(pointer).expect("serde paths produce valid JSON Pointers")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormDataLimits {
    max_bytes: usize,
    max_tokens: usize,
    max_depth: usize,
    max_nodes: usize,
    max_members: usize,
    max_collection_length: usize,
    max_scalar_bytes: usize,
    max_form_tree_nodes: usize,
    max_repeated_items: usize,
    max_active_edit_buffers: usize,
    max_edit_buffer_bytes: usize,
    max_total_edit_buffer_bytes: usize,
    max_host_operations_per_transaction: usize,
    max_retained_validation_findings: usize,
    max_validation_parameter_bytes: usize,
    max_canonical_integer_digits: usize,
}

impl FormDataLimits {
    pub fn max_bytes(mut self, maximum: usize) -> Self {
        self.max_bytes = maximum;
        self
    }

    pub fn max_tokens(mut self, maximum: usize) -> Self {
        self.max_tokens = maximum;
        self
    }

    pub fn max_depth(mut self, maximum: usize) -> Self {
        self.max_depth = maximum;
        self
    }

    pub fn max_nodes(mut self, maximum: usize) -> Self {
        self.max_nodes = maximum;
        self
    }

    pub fn max_members(mut self, maximum: usize) -> Self {
        self.max_members = maximum;
        self
    }

    pub fn max_collection_length(mut self, maximum: usize) -> Self {
        self.max_collection_length = maximum;
        self
    }

    pub fn max_scalar_bytes(mut self, maximum: usize) -> Self {
        self.max_scalar_bytes = maximum;
        self
    }

    pub fn max_form_tree_nodes(mut self, maximum: usize) -> Self {
        self.max_form_tree_nodes = maximum;
        self
    }

    pub fn max_repeated_items(mut self, maximum: usize) -> Self {
        self.max_repeated_items = maximum;
        self
    }

    pub fn max_active_edit_buffers(mut self, maximum: usize) -> Self {
        self.max_active_edit_buffers = maximum;
        self
    }

    pub fn max_edit_buffer_bytes(mut self, maximum: usize) -> Self {
        self.max_edit_buffer_bytes = maximum;
        self
    }

    pub fn max_total_edit_buffer_bytes(mut self, maximum: usize) -> Self {
        self.max_total_edit_buffer_bytes = maximum;
        self
    }

    pub fn max_host_operations_per_transaction(mut self, maximum: usize) -> Self {
        self.max_host_operations_per_transaction = maximum;
        self
    }

    pub fn max_retained_validation_findings(mut self, maximum: usize) -> Self {
        self.max_retained_validation_findings = maximum;
        self
    }

    pub fn max_validation_parameter_bytes(mut self, maximum: usize) -> Self {
        self.max_validation_parameter_bytes = maximum;
        self
    }

    pub fn max_canonical_integer_digits(mut self, maximum: usize) -> Self {
        self.max_canonical_integer_digits = maximum;
        self
    }

    pub(crate) fn input_limits(self) -> crate::limits::InputLimits {
        crate::limits::InputLimits {
            bytes: self.max_bytes,
            tokens: self.max_tokens,
            depth: self.max_depth,
            nodes: self.max_nodes,
            members: self.max_members,
            collection_length: self.max_collection_length,
            scalar_bytes: self.max_scalar_bytes,
        }
    }

    pub(crate) fn form_tree_nodes(self) -> usize {
        self.max_form_tree_nodes
    }

    pub(crate) fn repeated_items(self) -> usize {
        self.max_repeated_items
    }

    pub(crate) fn active_edit_buffers(self) -> usize {
        self.max_active_edit_buffers
    }

    pub(crate) fn edit_buffer_bytes(self) -> usize {
        self.max_edit_buffer_bytes
    }

    pub(crate) fn total_edit_buffer_bytes(self) -> usize {
        self.max_total_edit_buffer_bytes
    }

    pub(crate) fn host_operations_per_transaction(self) -> usize {
        self.max_host_operations_per_transaction
    }

    pub(crate) fn retained_validation_findings(self) -> usize {
        self.max_retained_validation_findings
    }

    pub(crate) fn validation_parameter_bytes(self) -> usize {
        self.max_validation_parameter_bytes
    }

    pub(crate) fn canonical_integer_digits(self) -> usize {
        self.max_canonical_integer_digits
    }
}

impl Default for FormDataLimits {
    fn default() -> Self {
        Self {
            max_bytes: 4 * 1_024 * 1_024,
            max_tokens: 262_144,
            max_depth: 32,
            max_nodes: 65_536,
            max_members: 65_536,
            max_collection_length: 1_024,
            max_scalar_bytes: 256 * 1_024,
            max_form_tree_nodes: 65_536,
            max_repeated_items: 16_384,
            max_active_edit_buffers: 1_024,
            max_edit_buffer_bytes: 512 * 1_024,
            max_total_edit_buffer_bytes: 4 * 1_024 * 1_024,
            max_host_operations_per_transaction: 256,
            max_retained_validation_findings: 256,
            max_validation_parameter_bytes: 4_096,
            max_canonical_integer_digits: 4_096,
        }
    }
}

/// An owned JSON syntax diagnostic.
///
/// Line and column use the one-based positions reported by `serde_json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonSyntaxError {
    line: usize,
    column: usize,
    reason: String,
}

impl JsonSyntaxError {
    fn from_serde_json(error: serde_json::Error) -> Self {
        let line = error.line();
        let column = error.column();
        let message = error.to_string();
        let suffix = format!(" at line {line} column {column}");
        let reason = message.strip_suffix(&suffix).unwrap_or(&message).to_owned();
        Self {
            line,
            column,
            reason,
        }
    }

    /// Returns the one-based line where `serde_json` detected the error.
    pub fn line(&self) -> usize {
        self.line
    }

    /// Returns the one-based column where `serde_json` detected the error.
    pub fn column(&self) -> usize {
        self.column
    }

    /// Returns `serde_json`'s human-readable reason without its location suffix.
    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl fmt::Display for JsonSyntaxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} at line {} column {}",
            self.reason, self.line, self.column
        )
    }
}

impl Error for JsonSyntaxError {}

/// A failure to parse or structurally bound a JSON input.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum JsonParseError {
    /// The input is not syntactically valid JSON.
    Syntax(JsonSyntaxError),
    /// The input exceeded a deterministic construction resource limit.
    ResourceLimit(ResourceLimitError),
    /// A data schema exceeded a compilation limit.
    CompilationLimit(crate::CompilationLimitError),
    /// A syntactically valid JSON value is not a valid UI schema.
    InvalidUiSchema {
        /// Location of the invalid value in the authored UI schema.
        location: JsonPointer,
        /// Human-readable diagnostic from strict wire decoding or post-decode validation.
        ///
        /// This text is intended for diagnostics and is not a stable machine-readable category.
        reason: String,
    },
}

impl fmt::Display for JsonParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(error) => write!(formatter, "invalid JSON: {error}"),
            Self::ResourceLimit(error) => error.fmt(formatter),
            Self::CompilationLimit(error) => error.fmt(formatter),
            Self::InvalidUiSchema { location, reason } => {
                write!(formatter, "invalid UI schema at {location}: {reason}")
            }
        }
    }
}

impl Error for JsonParseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Syntax(error) => Some(error),
            Self::ResourceLimit(error) => Some(error),
            Self::CompilationLimit(error) => Some(error),
            Self::InvalidUiSchema { .. } => None,
        }
    }
}
