//! Enforces the crate layering documented in the README: a crate may only
//! depend on Terra crates in strictly lower layers.

use std::path::Path;

/// Layer of every workspace crate. New crates must be added here.
const LAYERS: &[(&str, u8)] = &[
    ("terra-types", 0),
    ("terra-core", 0),
    ("terra-memory", 0),
    ("terra-providers", 0),
    ("terra-protocol", 1),
    ("terra-trace", 2),
    ("terra-compile", 2),
    ("terra-caging", 2),
    ("terra-executor", 3),
    ("terra-agents", 4),
    ("terra-mcp", 5),
    ("terra-cli", 6),
];

fn layer_of(name: &str) -> u8 {
    LAYERS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, l)| *l)
        .unwrap_or_else(|| panic!("crate {} has no layer in tests/layering.rs", name))
}

#[test]
fn crates_only_depend_on_lower_layers() {
    let crates_dir = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut checked = 0;
    for entry in std::fs::read_dir(crates_dir).unwrap() {
        let manifest_path = entry.unwrap().path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&manifest_path) else { continue };
        let manifest: toml::Table = text.parse().unwrap();
        let name = manifest["package"]["name"].as_str().unwrap().to_string();
        let layer = layer_of(&name);
        checked += 1;

        let Some(deps) = manifest.get("dependencies").and_then(|d| d.as_table()) else { continue };
        for dep in deps.keys().filter(|d| d.starts_with("terra-")) {
            assert!(
                layer_of(dep) < layer,
                "{} (layer {}) depends on {} (layer {})",
                name, layer, dep, layer_of(dep)
            );
        }
    }
    assert_eq!(checked, LAYERS.len(), "every workspace crate should be checked");
}
