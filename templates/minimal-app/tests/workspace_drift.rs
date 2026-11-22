//! 模板与根 workspace 的 drift 检查。
//!
//! 模板是独立 workspace；工具链与 dev-profile 若静默分叉，CI 的模板
//! 构建就会和根 workspace 验证不同的编译条件。模板被复制到仓库外时
//! 找不到根清单，检查直接跳过。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// 仓库根；`templates/minimal-app` 的测试工作目录是包根。
fn repo_root() -> Option<PathBuf> {
    let root = Path::new("../..");
    let manifest = fs::read_to_string(root.join("Cargo.toml")).ok()?;
    manifest
        .contains("name = \"zenui\"")
        .then(|| root.to_path_buf())
}

fn normalized(path: &Path) -> String {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `[profile.dev]` 及其所有子表的非注释行。
fn dev_profiles(manifest: &str) -> BTreeMap<String, Vec<String>> {
    let mut profiles = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(['[', ']']).to_owned();
            let is_dev = name == "profile.dev" || name.starts_with("profile.dev.");
            current = is_dev.then(|| {
                profiles.entry(name.clone()).or_insert_with(Vec::new);
                name
            });
        } else if let Some(name) = &current
            && !line.is_empty()
            && !line.starts_with('#')
        {
            profiles
                .entry(name.clone())
                .or_insert_with(Vec::new)
                .push(line.to_owned());
        }
    }
    profiles
}

#[test]
fn toolchain_matches_the_root_workspace() {
    let Some(root) = repo_root() else {
        return;
    };
    assert_eq!(
        normalized(Path::new("rust-toolchain.toml")),
        normalized(&root.join("rust-toolchain.toml")),
        "templates/minimal-app/rust-toolchain.toml drifted from the root workspace"
    );
}

#[test]
fn dev_profiles_match_the_root_workspace() {
    let Some(root) = repo_root() else {
        return;
    };
    let template = normalized(Path::new("Cargo.toml"));
    let root_manifest = normalized(&root.join("Cargo.toml"));
    assert_eq!(
        dev_profiles(&template),
        dev_profiles(&root_manifest),
        "templates/minimal-app dev profiles drifted from the root workspace"
    );
}

#[test]
fn dev_profile_parser_covers_every_dev_table() {
    let manifest = "\
[profile.dev]
opt-level = 1

[profile.dev.build-override]
opt-level = 2

[profile.dev.package.\"*\"]
opt-level = 3

[profile.release]
opt-level = 3
";
    let profiles = dev_profiles(manifest);
    assert_eq!(profiles["profile.dev"], vec!["opt-level = 1"]);
    assert_eq!(
        profiles["profile.dev.build-override"],
        vec!["opt-level = 2"]
    );
    assert_eq!(profiles["profile.dev.package.\"*\""], vec!["opt-level = 3"]);
    assert!(!profiles.contains_key("profile.release"));
}
