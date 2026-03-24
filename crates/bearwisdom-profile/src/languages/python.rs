use crate::types::*;

static PIP: PmDescriptor = PmDescriptor {
    name: "pip",
    lock_file: Some("requirements.txt"),
    deps_dir: Some(".venv"),
    install_cmd: ShellCommands {
        bash: "pip install -r requirements.txt",
        powershell: "pip install -r requirements.txt",
        cmd: "pip install -r requirements.txt",
    },
    restore_cmd: ShellCommands::same("pip install -r requirements.txt"),
};

static POETRY: PmDescriptor = PmDescriptor {
    name: "poetry",
    lock_file: Some("poetry.lock"),
    deps_dir: None,
    install_cmd: ShellCommands::same("poetry install"),
    restore_cmd: ShellCommands::same("poetry install --no-root"),
};

static PIPENV: PmDescriptor = PmDescriptor {
    name: "pipenv",
    lock_file: Some("Pipfile.lock"),
    deps_dir: None,
    install_cmd: ShellCommands::same("pipenv install"),
    restore_cmd: ShellCommands::same("pipenv sync"),
};

static UV: PmDescriptor = PmDescriptor {
    name: "uv",
    lock_file: Some("uv.lock"),
    deps_dir: Some(".venv"),
    install_cmd: ShellCommands::same("uv sync"),
    restore_cmd: ShellCommands::same("uv sync --frozen"),
};

static PYTEST: TfDescriptor = TfDescriptor {
    name: "pytest",
    display_name: "pytest",
    config_files: &["pytest.ini", "pyproject.toml", "setup.cfg", "tox.ini"],
    config_content_match: Some("[tool.pytest"),
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands::same("python -m pytest --collect-only -q")),
    run_cmd: ShellCommands::same("python -m pytest"),
    run_single_cmd: ShellCommands::same("python -m pytest {file}"),
};

static UNITTEST: TfDescriptor = TfDescriptor {
    name: "unittest",
    display_name: "unittest",
    config_files: &[],
    config_content_match: Some("import unittest"),
    package_json_dep: None,
    discovery_cmd: Some(ShellCommands::same("python -m unittest discover")),
    run_cmd: ShellCommands::same("python -m unittest discover"),
    run_single_cmd: ShellCommands::same("python -m unittest {file}"),
};

static VENV_RESTORE: RestoreStep = RestoreStep {
    id: "python-venv",
    title: "Create Python virtual environment",
    description: "Create a venv and install dependencies with pip.",
    trigger: RestoreTrigger::DirMissing,
    watch_path: ".venv",
    commands: ShellCommands {
        bash: "python -m venv .venv && source .venv/bin/activate && pip install -r requirements.txt",
        powershell: "python -m venv .venv; .venv\\Scripts\\Activate.ps1; pip install -r requirements.txt",
        cmd: "python -m venv .venv && .venv\\Scripts\\activate.bat && pip install -r requirements.txt",
    },
    auto_fixable: true,
    critical: true,
};

pub static PYTHON: LanguageDescriptor = LanguageDescriptor {
    id: "python",
    display_name: "Python",
    file_extensions: &[".py", ".pyi", ".pyw"],
    filenames: &[],
    aliases: &["py"],
    exclude_dirs: &[
        ".venv", "venv", "env", "__pycache__", ".pytest_cache",
        ".mypy_cache", ".ruff_cache", "dist", "build",
    ],
    entry_point_files: &[
        "pyproject.toml", "requirements.txt", "setup.py", "setup.cfg",
        "Pipfile", "poetry.lock", "uv.lock",
    ],
    sdk: Some(SdkDescriptor {
        name: "Python",
        version_command: "python",
        version_args: &["--version"],
        version_file: Some(".python-version"),
        version_json_key: None,
        install_url: "https://www.python.org/downloads/",
    }),
    package_managers: &[PIP, POETRY, PIPENV, UV],
    test_frameworks: &[PYTEST, UNITTEST],
    restore_steps: &[VENV_RESTORE],
    line_comment: Some("#"),
    block_comment: Some(("\"\"\"", "\"\"\"")),
};
