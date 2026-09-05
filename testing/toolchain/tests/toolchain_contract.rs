//! Toolchain contract: one Dioxus pin and one minimum supported Rust version
//! across every manifest in the repository.
//!
//! The demo is a separate Cargo workspace with its own lockfile and `fuzz/` is
//! excluded from the root workspace, so neither can inherit the Dioxus pin or
//! `rust-version` from `[workspace.package]`. These tests keep the three
//! manifests from drifting apart: a partial dependency bump once left the
//! Dioxus umbrella crate on one patch release while its core crates moved to
//! the next, and the demo lockfile fell behind the adapter it depends on.
//!
//! This is repository infrastructure, not part of any product crate's
//! contract: it reads files outside the crate it would ship in, so it lives in
//! an unpublished `testing/` package rather than under `crates/*/tests/`.

const WORKSPACE_MANIFEST: &str = include_str!("../../../Cargo.toml");
const WORKSPACE_LOCK: &str = include_str!("../../../Cargo.lock");
const FUZZ_MANIFEST: &str = include_str!("../../../fuzz/Cargo.toml");
const DEMO_MANIFEST: &str = include_str!("../../../demo/Cargo.toml");
const DEMO_LOCK: &str = include_str!("../../../demo/Cargo.lock");
const DEMO_README: &str = include_str!("../../../demo/README.md");

/// Workspace dependency keys the root pins exactly, paired with the crate each
/// key resolves to in a lockfile (`dioxus-elements` renames `dioxus-html`).
const PINNED_DIOXUS_DEPENDENCIES: [(&str, &str); 5] = [
    ("dioxus", "dioxus"),
    ("dioxus-core", "dioxus-core"),
    ("dioxus-elements", "dioxus-html"),
    ("dioxus-signals", "dioxus-signals"),
    ("dioxus-web", "dioxus-web"),
];

/// Returns the version requirement of a `[dependencies]`-style entry, from
/// either the shorthand `key = "req"` or the table form `key = { version = "req", .. }`.
fn dependency_requirement<'a>(manifest: &'a str, key: &str) -> Option<&'a str> {
    let prefix = format!("{key} = ");
    let entry = manifest
        .lines()
        .find_map(|line| line.strip_prefix(prefix.as_str()))?;
    let value = match entry.strip_prefix('"') {
        Some(shorthand) => shorthand,
        None => entry.split_once("version = \"")?.1,
    };
    value.split_once('"').map(|(requirement, _)| requirement)
}

/// Returns the exact version a dependency is pinned to, failing if it is not an
/// `=x.y.z` requirement.
fn exact_pin(manifest: &str, key: &str) -> String {
    let requirement = dependency_requirement(manifest, key)
        .unwrap_or_else(|| panic!("the manifest should declare {key}"));
    requirement
        .strip_prefix('=')
        .unwrap_or_else(|| panic!("{key} should be pinned exactly, found {requirement:?}"))
        .to_owned()
}

fn locked_version<'a>(lock_name: &str, lock: &'a str, package: &str) -> Option<&'a str> {
    let mut entries = lock.split("[[package]]").filter(|entry| {
        entry
            .lines()
            .any(|line| line == format!("name = \"{package}\""))
    });
    let entry = entries.next()?;
    assert!(
        entries.next().is_none(),
        "the {lock_name} lock must contain exactly one {package} version"
    );
    entry
        .lines()
        .find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
}

fn rust_version(manifest: &str) -> &str {
    manifest
        .lines()
        .find_map(|line| line.strip_prefix("rust-version = \"")?.strip_suffix('"'))
        .expect("the manifest should declare rust-version")
}

#[test]
fn dioxus_crates_share_one_exact_pin_across_both_workspaces() {
    let pin = exact_pin(WORKSPACE_MANIFEST, "dioxus");

    for (key, _) in PINNED_DIOXUS_DEPENDENCIES {
        assert_eq!(
            exact_pin(WORKSPACE_MANIFEST, key),
            pin,
            "workspace dependency {key} should share the dioxus pin"
        );
    }
    assert_eq!(
        exact_pin(DEMO_MANIFEST, "dioxus"),
        pin,
        "the demo should pin the same dioxus release as the root workspace"
    );

    for (lock_name, lock) in [("root", WORKSPACE_LOCK), ("demo", DEMO_LOCK)] {
        for (_, package) in PINNED_DIOXUS_DEPENDENCIES {
            assert_eq!(
                locked_version(lock_name, lock, package),
                Some(pin.as_str()),
                "the {lock_name} lock should resolve {package} to the pinned release"
            );
        }
    }

    // The demo's Dagger pipeline installs the `dx` CLI matching the locked
    // `dioxus` version; the README must tell contributors to do the same.
    assert!(
        DEMO_README.contains(&format!(
            "cargo install dioxus-cli --version {pin} --locked"
        )),
        "the demo README should install the dioxus-cli release matching the pin"
    );
}

#[test]
fn minimum_supported_rust_version_is_declared_identically_in_the_root_fuzz_and_demo_manifests() {
    let workspace_rust_version = rust_version(WORKSPACE_MANIFEST);

    for (name, manifest) in [("fuzz", FUZZ_MANIFEST), ("demo", DEMO_MANIFEST)] {
        assert_eq!(
            rust_version(manifest),
            workspace_rust_version,
            "the {name} manifest cannot inherit rust-version and must restate the workspace value"
        );
    }
}
