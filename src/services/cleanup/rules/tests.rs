// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::collections::HashSet;
use std::path::Path;

fn names(list: &[&str]) -> HashSet<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn kind_of(path: &str, dir: &str, siblings: &[&str]) -> Option<&'static str> {
    classify(Path::new(path), dir, &names(siblings)).map(|rule| rule.kind)
}

#[test]
fn node_modules_needs_no_marker() {
    assert_eq!(
        kind_of("/p/node_modules", "node_modules", &[]),
        Some("npm packages")
    );
}

#[test]
fn target_requires_a_manifest() {
    assert_eq!(
        kind_of("/p/target", "target", &["Cargo.toml"]),
        Some("Rust build output")
    );
    assert_eq!(
        kind_of("/p/target", "target", &["pom.xml"]),
        Some("Maven build output")
    );
    assert_eq!(kind_of("/photos/target", "target", &["notes.txt"]), None);
}

#[test]
fn ambiguous_names_need_project_markers() {
    assert_eq!(
        kind_of("/p/build", "build", &["CMakeLists.txt"]),
        Some("CMake/Meson build dir")
    );
    assert_eq!(
        kind_of("/p/build", "build", &["build.gradle"]),
        Some("Gradle build output")
    );
    assert_eq!(kind_of("/holiday/build", "build", &["img.png"]), None);
    assert_eq!(
        kind_of("/p/dist", "dist", &["package.json"]),
        Some("build output")
    );
    assert_eq!(kind_of("/p/dist", "dist", &[]), None);
}

#[test]
fn unity_needs_both_markers() {
    assert_eq!(
        kind_of("/g/Library", "Library", &["Assets", "ProjectSettings"]),
        Some("Unity library cache")
    );
    assert_eq!(kind_of("/g/Library", "Library", &["Assets"]), None);
}

#[test]
fn dotnet_matches_by_extension() {
    assert_eq!(
        kind_of("/p/obj", "obj", &["App.csproj"]),
        Some(".NET intermediates")
    );
    assert_eq!(kind_of("/p/obj", "obj", &["readme.md"]), None);
}

#[test]
fn suffix_and_prefix_rules() {
    assert_eq!(
        kind_of("/p/mylib.egg-info", "mylib.egg-info", &[]),
        Some("package metadata")
    );
    assert_eq!(
        kind_of("/p/cmake-build-debug", "cmake-build-debug", &[]),
        Some("CLion build dir")
    );
    assert_eq!(kind_of("/p/.egg-info", ".egg-info", &[]), None);
}

#[test]
fn path_rules_match_tool_caches() {
    assert_eq!(
        kind_of("/home/x/.cargo/registry", "registry", &[]),
        Some("cargo registry cache")
    );
    assert_eq!(kind_of("/srv/registry", "registry", &[]), None);
}

#[test]
fn most_confident_rule_wins() {
    let rule = classify(
        Path::new("/p/build"),
        "build",
        &names(&["build.gradle", "CMakeLists.txt"]),
    )
    .expect("gradle build should match");
    assert_eq!(rule.confidence, Confidence::High);
}
