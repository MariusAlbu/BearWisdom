use crate::types::*;

pub static DOCKERFILE: LanguageDescriptor = LanguageDescriptor {
    id: "dockerfile",
    display_name: "Dockerfile",
    file_extensions: &[".dockerfile"],
    filenames: &["Dockerfile", "Containerfile"],
    aliases: &["docker"],
    exclude_dirs: &[],
    entry_point_files: &["Dockerfile", "docker-compose.yml", "docker-compose.yaml", ".dockerignore"],
    sdk: Some(SdkDescriptor {
        name: "Docker",
        version_command: "docker",
        version_args: &["--version"],
        version_file: None,
        version_json_key: None,
        install_url: "https://docs.docker.com/get-docker/",
    }),
    package_managers: &[],
    test_frameworks: &[],
    restore_steps: &[],
    line_comment: Some("#"),
    block_comment: None,
};
