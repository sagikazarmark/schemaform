use schemaform::{FormCompiler, FormDefinition, RetrievalUri, SchemaResource};
use serde_json::Value;

pub const MANIFEST_SOURCE: &str = include_str!("manifest.json");

const RESOURCE_SOURCES: &[(&str, &str)] = &[
    (
        "fixtures/bods-address-international/country.json",
        include_str!("fixtures/bods-address-international/country.json"),
    ),
    (
        "fixtures/bods-address-international/schema.json",
        include_str!("fixtures/bods-address-international/schema.json"),
    ),
    (
        "fixtures/bods-entity-onboarding/schema.json",
        include_str!("fixtures/bods-entity-onboarding/schema.json"),
    ),
    (
        "fixtures/bods-person-onboarding/schema.json",
        include_str!("fixtures/bods-person-onboarding/schema.json"),
    ),
    (
        "fixtures/gobl-invoice-billing/schema.json",
        include_str!("fixtures/gobl-invoice-billing/schema.json"),
    ),
    (
        "fixtures/gobl-invoice-line-items/schema.json",
        include_str!("fixtures/gobl-invoice-line-items/schema.json"),
    ),
    (
        "fixtures/gobl-payment-terms/schema.json",
        include_str!("fixtures/gobl-payment-terms/schema.json"),
    ),
    (
        "fixtures/hubverse-account-settings/schema.json",
        include_str!("fixtures/hubverse-account-settings/schema.json"),
    ),
    (
        "fixtures/openkyc-communication-preferences/schema.json",
        include_str!("fixtures/openkyc-communication-preferences/schema.json"),
    ),
    (
        "fixtures/openkyc-consent-preferences/schema.json",
        include_str!("fixtures/openkyc-consent-preferences/schema.json"),
    ),
    (
        "fixtures/openkyc-customer-onboarding/schema.json",
        include_str!("fixtures/openkyc-customer-onboarding/schema.json"),
    ),
    (
        "fixtures/openkyc-marketing-preferences/schema.json",
        include_str!("fixtures/openkyc-marketing-preferences/schema.json"),
    ),
    (
        "fixtures/ucp-billing-address/schema.json",
        include_str!("fixtures/ucp-billing-address/schema.json"),
    ),
    (
        "fixtures/ucp-buyer-account/schema.json",
        include_str!("fixtures/ucp-buyer-account/schema.json"),
    ),
    (
        "fixtures/ucp-card-payment/schema.json",
        include_str!("fixtures/ucp-card-payment/schema.json"),
    ),
    (
        "fixtures/ucp-cart-line-items/schema.json",
        include_str!("fixtures/ucp-cart-line-items/schema.json"),
    ),
    (
        "fixtures/ucp-order-line-items/schema.json",
        include_str!("fixtures/ucp-order-line-items/schema.json"),
    ),
    (
        "fixtures/ucp-payment-instrument/credential.json",
        include_str!("fixtures/ucp-payment-instrument/credential.json"),
    ),
    (
        "fixtures/ucp-payment-instrument/postal-address.json",
        include_str!("fixtures/ucp-payment-instrument/postal-address.json"),
    ),
    (
        "fixtures/ucp-payment-instrument/schema.json",
        include_str!("fixtures/ucp-payment-instrument/schema.json"),
    ),
    (
        "fixtures/ucp-product-catalog/schema.json",
        include_str!("fixtures/ucp-product-catalog/schema.json"),
    ),
    (
        "fixtures/ucp-product-option/schema.json",
        include_str!("fixtures/ucp-product-option/schema.json"),
    ),
    (
        "fixtures/ucp-product-variant/schema.json",
        include_str!("fixtures/ucp-product-variant/schema.json"),
    ),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedCapabilityFinding {
    pub code: String,
    pub instance_location: String,
    pub resource_uri: String,
    pub keyword_pointer: String,
    pub parameters: Value,
    pub blocking: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpectedControl {
    pub binding: String,
    pub kind: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GenerationOutcome {
    InProfile,
    CapabilityBlocked,
}

#[derive(Clone)]
pub struct BusinessSchemaFixture {
    pub id: String,
    generation: GenerationOutcome,
    root_uri: RetrievalUri,
    root_schema: Value,
    supporting_resources: Vec<SchemaResource>,
    pub expected_controls: Vec<ExpectedControl>,
    #[allow(dead_code)]
    pub expected_findings: Vec<ExpectedCapabilityFinding>,
}

impl BusinessSchemaFixture {
    pub fn is_in_profile(&self) -> bool {
        self.generation == GenerationOutcome::InProfile
    }

    pub fn compiler(&self) -> FormCompiler {
        self.supporting_resources.iter().cloned().fold(
            FormDefinition::compiler(self.root_schema.clone()).root_uri(self.root_uri.clone()),
            FormCompiler::resource,
        )
    }
}

pub fn fixtures() -> Vec<BusinessSchemaFixture> {
    let manifest: Value =
        serde_json::from_str(MANIFEST_SOURCE).expect("the embedded corpus manifest should parse");
    manifest["fixtures"]
        .as_array()
        .expect("the corpus manifest should contain fixtures")
        .iter()
        .map(load_fixture)
        .collect()
}

fn load_fixture(fixture: &Value) -> BusinessSchemaFixture {
    let id = string(fixture, "id").to_owned();
    let resources = fixture["resources"]
        .as_array()
        .unwrap_or_else(|| panic!("fixture {id} should declare resources"));
    let resource_uri = |name: &str| {
        resources
            .iter()
            .find(|resource| string(resource, "name") == name)
            .map(|resource| string(resource, "retrieval_uri"))
            .unwrap_or_else(|| panic!("fixture {id} should declare resource {name}"))
    };
    let root = resources
        .iter()
        .find(|resource| resource["role"] == "root")
        .unwrap_or_else(|| panic!("fixture {id} should declare a root resource"));
    let root_uri = uri(string(root, "retrieval_uri"));
    let root_schema = parse_resource(string(root, "path"));
    let supporting_resources = resources
        .iter()
        .filter(|resource| resource["role"] != "root")
        .map(|resource| {
            SchemaResource::new(
                uri(string(resource, "retrieval_uri")),
                parse_resource(string(resource, "path")),
            )
        })
        .collect();
    let expected_findings = fixture["expected"]["capability_findings"]
        .as_array()
        .unwrap_or_else(|| panic!("fixture {id} should declare capability findings"))
        .iter()
        .map(|finding| {
            let location = string(finding, "schema_location");
            let (resource_name, keyword_pointer) = location
                .split_once('#')
                .unwrap_or_else(|| panic!("fixture {id} has invalid schema location {location}"));
            ExpectedCapabilityFinding {
                code: string(finding, "profile_id").to_owned(),
                instance_location: string(finding, "instance_location").to_owned(),
                resource_uri: resource_uri(resource_name).to_owned(),
                keyword_pointer: keyword_pointer.to_owned(),
                parameters: finding["parameters"].clone(),
                blocking: string(finding, "classification") == "capability-blocking",
            }
        })
        .collect::<Vec<_>>();
    let expected_controls = fixture["expected"]["controls"]
        .as_array()
        .unwrap_or_else(|| panic!("fixture {id} should declare expected controls"))
        .iter()
        .map(|control| ExpectedControl {
            binding: string(control, "binding").to_owned(),
            kind: string(control, "kind").to_owned(),
        })
        .collect();

    let generation = match string(&fixture["expected"], "generation_alone") {
        "sufficient" | "authored-ui-required" => GenerationOutcome::InProfile,
        "capability-blocked" => GenerationOutcome::CapabilityBlocked,
        generation => panic!("fixture {id} has unknown generation outcome {generation}"),
    };

    BusinessSchemaFixture {
        id,
        generation,
        root_uri,
        root_schema,
        supporting_resources,
        expected_controls,
        expected_findings,
    }
}

fn parse_resource(path: &str) -> Value {
    let source = RESOURCE_SOURCES
        .iter()
        .find_map(|(candidate, source)| (*candidate == path).then_some(*source))
        .unwrap_or_else(|| panic!("embedded corpus resource is missing: {path}"));
    serde_json::from_str(source)
        .unwrap_or_else(|error| panic!("embedded corpus resource {path} should parse: {error}"))
}

fn uri(value: &str) -> RetrievalUri {
    RetrievalUri::parse(value)
        .unwrap_or_else(|_| panic!("corpus retrieval URI should be absolute: {value}"))
}

fn string<'a>(value: &'a Value, field: &str) -> &'a str {
    value[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} should be a string"))
}
