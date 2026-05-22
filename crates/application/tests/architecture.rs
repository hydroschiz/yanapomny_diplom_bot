use std::{collections::BTreeSet, fs, path::PathBuf};

#[test]
fn cargo_dependencies_preserve_layer_boundaries() {
    let root = workspace_root();
    let rules: &[(&str, &[&str])] = &[
        (
            "crates/shared",
            &[
                "domain",
                "application",
                "infrastructure",
                "presentation",
                "transport-core",
                "transport-vk",
                "mongodb",
                "redis",
                "reqwest",
                "axum",
                "vk-bot-api",
            ],
        ),
        (
            "crates/domain",
            &[
                "application",
                "infrastructure",
                "presentation",
                "transport-core",
                "transport-vk",
                "mongodb",
                "redis",
                "reqwest",
                "axum",
                "vk-bot-api",
            ],
        ),
        (
            "crates/application",
            &[
                "infrastructure",
                "presentation",
                "transport-core",
                "transport-vk",
                "mongodb",
                "redis",
                "reqwest",
                "axum",
                "vk-bot-api",
            ],
        ),
        (
            "crates/presentation",
            &[
                "infrastructure",
                "transport-vk",
                "mongodb",
                "redis",
                "reqwest",
                "axum",
                "vk-bot-api",
            ],
        ),
        (
            "crates/transport-core",
            &[
                "domain",
                "application",
                "infrastructure",
                "presentation",
                "transport-vk",
                "mongodb",
                "redis",
                "reqwest",
                "axum",
                "vk-bot-api",
            ],
        ),
        (
            "crates/transport-vk",
            &[
                "domain",
                "application",
                "infrastructure",
                "presentation",
                "mongodb",
                "redis",
                "reqwest",
                "axum",
            ],
        ),
        (
            "crates/infrastructure",
            &[
                "presentation",
                "transport-core",
                "transport-vk",
                "axum",
                "vk-bot-api",
            ],
        ),
        (
            "bins/bot",
            &["mongodb", "redis", "reqwest", "axum", "vk-bot-api"],
        ),
    ];

    let mut violations = Vec::new();
    for (crate_path, forbidden) in rules {
        let dependencies = dependency_names(root.join(crate_path).join("Cargo.toml"));
        for dependency in *forbidden {
            if dependencies.contains(*dependency) {
                violations.push(format!(
                    "{crate_path} depends on forbidden crate `{dependency}`"
                ));
            }
        }
    }

    assert_no_violations(violations);
}

#[test]
fn source_imports_preserve_layer_boundaries() {
    let rules: &[(&str, &[&str])] = &[
        (
            "crates/shared/src",
            &[
                "domain::",
                "application::",
                "infrastructure::",
                "presentation::",
                "transport_core::",
                "transport_vk::",
                "mongodb::",
                "redis::",
                "reqwest::",
                "axum::",
                "vk_bot_api::",
            ],
        ),
        (
            "crates/domain/src",
            &[
                "application::",
                "infrastructure::",
                "presentation::",
                "transport_core::",
                "transport_vk::",
                "mongodb::",
                "redis::",
                "reqwest::",
                "axum::",
                "vk_bot_api::",
            ],
        ),
        (
            "crates/application/src",
            &[
                "infrastructure::",
                "presentation::",
                "transport_core::",
                "transport_vk::",
                "mongodb::",
                "redis::",
                "reqwest::",
                "axum::",
                "vk_bot_api::",
            ],
        ),
        (
            "crates/presentation/src",
            &[
                "infrastructure::",
                "transport_vk::",
                "mongodb::",
                "redis::",
                "reqwest::",
                "axum::",
                "vk_bot_api::",
            ],
        ),
        (
            "crates/transport-core/src",
            &[
                "domain::",
                "application::",
                "infrastructure::",
                "presentation::",
                "transport_vk::",
                "mongodb::",
                "redis::",
                "reqwest::",
                "axum::",
                "vk_bot_api::",
            ],
        ),
        (
            "crates/transport-vk/src",
            &[
                "domain::",
                "application::",
                "infrastructure::",
                "presentation::",
                "mongodb::",
                "redis::",
                "reqwest::",
                "axum::",
            ],
        ),
        (
            "crates/infrastructure/src",
            &[
                "presentation::",
                "transport_core::",
                "transport_vk::",
                "axum::",
                "vk_bot_api::",
            ],
        ),
        (
            "bins/bot/src",
            &[
                "mongodb::",
                "redis::",
                "reqwest::",
                "axum::",
                "vk_bot_api::",
            ],
        ),
    ];

    let root = workspace_root();
    let mut violations = Vec::new();
    for (source_path, forbidden) in rules {
        for file in rust_files(root.join(source_path)) {
            let content = fs::read_to_string(&file).expect("failed to read Rust source");
            for (line_index, line) in content.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }

                for token in *forbidden {
                    if line.contains(token) {
                        let path = file.strip_prefix(&root).unwrap_or(&file);
                        violations.push(format!(
                            "{}:{} references forbidden token `{}`",
                            path.display(),
                            line_index + 1,
                            token
                        ));
                    }
                }
            }
        }
    }

    assert_no_violations(violations);
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("application crate should be under crates/")
        .to_path_buf()
}

fn dependency_names(cargo_toml: PathBuf) -> BTreeSet<String> {
    let content = fs::read_to_string(cargo_toml).expect("failed to read Cargo.toml");
    let mut dependencies = BTreeSet::new();
    let mut in_dependencies = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dependencies = trimmed == "[dependencies]";
            continue;
        }
        if !in_dependencies || trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = trimmed.split_once('=') {
            dependencies.insert(name.trim().trim_matches('"').to_string());
        }
    }

    dependencies
}

fn rust_files(root: PathBuf) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files.sort();
    files
}

fn collect_rust_files(path: PathBuf, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(path).expect("failed to read source directory") {
        let path = entry.expect("failed to read directory entry").path();
        if path.is_dir() {
            collect_rust_files(path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn assert_no_violations(violations: Vec<String>) {
    assert!(
        violations.is_empty(),
        "architecture boundary violations:\n{}",
        violations.join("\n")
    );
}
