use crate::types::EnvironmentInfo;
use std::path::Path;

/// Detect .env file status and docker-compose presence.
pub fn detect_environment(root: &Path) -> EnvironmentInfo {
    let has_env_example = root.join(".env.example").exists()
        || root.join(".env.example.local").exists()
        || root.join(".env.sample").exists();

    let has_env = root.join(".env").exists() || root.join(".env.local").exists();

    let missing_env_file = has_env_example && !has_env;

    let has_docker_compose = root.join("docker-compose.yml").exists()
        || root.join("docker-compose.yaml").exists()
        || root.join("compose.yml").exists()
        || root.join("compose.yaml").exists();

    // Collect all .env* files (exclude .env.example variants).
    let mut env_files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with(".env") && !name.contains("example") && !name.contains("sample") {
                env_files.push(name);
            }
        }
    }
    env_files.sort();

    EnvironmentInfo {
        missing_env_file,
        has_docker_compose,
        env_files,
    }
}
