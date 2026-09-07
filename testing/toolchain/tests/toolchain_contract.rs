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

/// Dioxus crates only the demo resolves, through its `desktop` feature. The
/// umbrella crate requires them with a caret range, so they can drift to a
/// later patch release the way the core crates once did.
const DEMO_ONLY_DIOXUS_PACKAGES: [&str; 1] = ["dioxus-desktop"];

/// Cargo feature names that `dx` 0.7 reads as a renderer selection when they
/// appear on the `dioxus` dependency or in a package feature.
const DIOXUS_RENDERER_FEATURES: [&str; 6] =
    ["web", "desktop", "mobile", "native", "server", "liveview"];

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

/// Returns the lines of the `[name]` table: those after its header up to the
/// next table header.
fn table<'a>(manifest: &'a str, name: &str) -> Option<impl Iterator<Item = &'a str>> {
    let header = format!("[{name}]");
    let mut lines = manifest.lines().skip_while(move |line| *line != header);
    lines.next()?;
    Some(lines.take_while(|line| !line.starts_with('[')))
}

/// Collects the string items of an array whose opening `[` has already been
/// consumed, reading `lines` up to the closing `]`. Items may sit on one line
/// or several, with `#` comments between them.
fn collect_string_items<'a>(lines: impl Iterator<Item = &'a str>) -> Option<Vec<String>> {
    let mut items = Vec::new();
    for line in lines {
        let code = line.split_once('#').map_or(line, |(code, _)| code);
        let (code, closed) = match code.split_once(']') {
            Some((code, _)) => (code, true),
            None => (code, false),
        };
        items.extend(
            code.split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| item.trim_matches('"').to_owned()),
        );
        if closed {
            return Some(items);
        }
    }
    None
}

/// Returns the string items of the array a line `key = [` opens in `lines`.
fn string_array<'a>(lines: impl Iterator<Item = &'a str>, key: &str) -> Option<Vec<String>> {
    let prefix = format!("{key} = [");
    let mut lines = lines.skip_while(|line| !line.starts_with(prefix.as_str()));
    let first = lines.next()?.strip_prefix(prefix.as_str())?;
    collect_string_items(std::iter::once(first).chain(lines))
}

/// Returns the `features` array of the `[dependencies]` entry `key`, written in
/// table form with the array opened on the entry's own line. The entry is matched
/// at the start of its line, so `dioxus` does not match `schemaform-dioxus`.
fn dependency_features(manifest: &str, key: &str) -> Option<Vec<String>> {
    let entry = format!("{key} = {{");
    let mut lines =
        table(manifest, "dependencies")?.skip_while(|line| !line.starts_with(entry.as_str()));
    let first = lines.next()?.split_once("features = [")?.1;
    collect_string_items(std::iter::once(first).chain(lines))
}

#[test]
fn the_demo_declares_one_cargo_feature_per_renderer_so_dx_selects_the_platform() {
    // `dx` always builds with `--no-default-features`, then adds the package
    // feature named after the platform it was asked for (`web`, `desktop`) and
    // every default feature that does not enable a renderer. `dx serve` with no
    // platform autodetects from a `default` feature that enables exactly one
    // renderer. A renderer feature spelled directly on the `dioxus` dependency
    // is compiled into every platform build instead, so it must not be there.
    let dioxus_features = dependency_features(DEMO_MANIFEST, "dioxus")
        .expect("the demo's dioxus dependency lists features");
    for feature in DIOXUS_RENDERER_FEATURES {
        assert!(
            !dioxus_features.iter().any(|enabled| enabled == feature),
            "the demo's dioxus dependency should not select the {feature} renderer for every platform build"
        );
    }

    let features = || table(DEMO_MANIFEST, "features").expect("the demo declares [features]");
    assert_eq!(
        string_array(features(), "default").as_deref(),
        Some(&["web".to_owned()][..]),
        "the demo's default feature should select the web renderer, so `dx serve` and `cargo test` stay web builds"
    );
    assert_eq!(
        string_array(features(), "web").as_deref(),
        Some(&["dioxus/web".to_owned()][..]),
        "the demo's web feature should enable exactly dioxus/web"
    );
    assert_eq!(
        string_array(features(), "desktop").as_deref(),
        Some(&["dioxus/desktop".to_owned()][..]),
        "the demo's desktop feature should enable exactly dioxus/desktop, so `dx serve --platform desktop` selects it"
    );
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
    for package in DEMO_ONLY_DIOXUS_PACKAGES {
        assert_eq!(
            locked_version("demo", DEMO_LOCK, package),
            Some(pin.as_str()),
            "the demo lock should resolve {package} to the pinned release"
        );
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
