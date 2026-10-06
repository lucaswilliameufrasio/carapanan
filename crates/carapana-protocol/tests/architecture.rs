use std::process::Command;

#[test]
fn should_keep_protocol_and_core_independent_of_terminal_and_web_interfaces() {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--locked", "--format-version", "1"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for name in ["carapana-protocol", "carapana-core"] {
        let package = metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|package| package["name"] == name)
            .unwrap();
        for dependency in package["dependencies"].as_array().unwrap() {
            if dependency.get("path").is_some() {
                assert!(
                    name == "carapana-core" && dependency["name"] == "carapana-protocol",
                    "Unexpected local dependency in {name}: {dependency}"
                );
            }
            assert!(
                !["ratatui", "crossterm", "carapana-ui-prototype"]
                    .contains(&dependency["name"].as_str().unwrap())
            );
        }
    }
}
