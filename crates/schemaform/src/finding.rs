use serde_json::Value;

use crate::{
    JsonPointer, SchemaLocation,
    definition::CapabilityFinding,
    form::{DataRevision, IndeterminateReason, InstanceIdentity},
};

/// One validator-produced explanation of an invalid validation outcome.
///
/// `code` is the stable JSON Schema keyword name. `instance_location` points
/// into form data, `keyword_location` identifies the qualified data-schema
/// keyword, and `parameters` contains the code-specific structured values.
/// Presentation text is deliberately supplied by the adapter or host rather
/// than stored in this core finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationFinding {
    code: String,
    instance_location: JsonPointer,
    keyword_location: SchemaLocation,
    parameters: Value,
}

impl ValidationFinding {
    pub(crate) fn new(
        code: impl Into<String>,
        instance_location: JsonPointer,
        keyword_location: SchemaLocation,
        parameters: Value,
    ) -> Self {
        Self {
            code: code.into(),
            instance_location,
            keyword_location,
            parameters,
        }
    }

    /// Returns the stable keyword code used to select presentation and parameters.
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the location of the invalid value in form data.
    pub fn instance_location(&self) -> &JsonPointer {
        &self.instance_location
    }

    /// Returns the canonical resource and pointer of the failing data-schema keyword.
    pub fn keyword_location(&self) -> &SchemaLocation {
        &self.keyword_location
    }

    /// Returns keyword-specific structured parameters for host-owned presentation.
    pub fn parameters(&self) -> &Value {
        &self.parameters
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFinding {
    pub(crate) code: String,
    pub(crate) instance_location: JsonPointer,
    pub(crate) parameters: Value,
    pub(crate) blocking: bool,
}

impl ExternalFinding {
    pub fn blocking(code: impl Into<String>, location: JsonPointer, parameters: Value) -> Self {
        Self {
            code: code.into(),
            instance_location: location,
            parameters,
            blocking: true,
        }
    }

    pub fn advisory(code: impl Into<String>, location: JsonPointer, parameters: Value) -> Self {
        Self {
            code: code.into(),
            instance_location: location,
            parameters,
            blocking: false,
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn instance_location(&self) -> &JsonPointer {
        &self.instance_location
    }

    pub fn parameters(&self) -> &Value {
        &self.parameters
    }

    pub fn is_blocking(&self) -> bool {
        self.blocking
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalFindingBatch {
    pub(crate) source: String,
    pub(crate) revision: DataRevision,
    pub(crate) findings: Vec<ExternalFinding>,
}

impl ExternalFindingBatch {
    pub fn new(
        source: impl Into<String>,
        revision: DataRevision,
        findings: impl Into<Vec<ExternalFinding>>,
    ) -> Self {
        Self {
            source: source.into(),
            revision,
            findings: findings.into(),
        }
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn data_revision(&self) -> DataRevision {
        self.revision
    }

    pub fn findings(&self) -> impl Iterator<Item = &ExternalFinding> {
        self.findings.iter()
    }
}

#[derive(Clone, Copy)]
#[non_exhaustive]
pub enum FindingView<'a> {
    Validation {
        target: InstanceIdentity,
        finding: &'a ValidationFinding,
    },
    ValidationFindingsTruncated {
        target: InstanceIdentity,
        retained: usize,
    },
    Indeterminate {
        target: InstanceIdentity,
        reason: &'a IndeterminateReason,
    },
    Capability {
        target: InstanceIdentity,
        finding: &'a CapabilityFinding,
    },
    External {
        target: InstanceIdentity,
        source: &'a str,
        finding: &'a ExternalFinding,
    },
    Parse {
        target: InstanceIdentity,
        kind: ParseBlockerKind,
    },
}

pub use crate::form::ParseBlockerKind;
