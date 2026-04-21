//! Shared test fixtures for BearWisdom integration tests.

use std::fs;
use std::path::Path;
use tempfile::TempDir;

use bearwisdom::Database;

/// A temporary project directory pre-populated with source files.
pub struct TestProject {
    pub dir: TempDir,
}

impl TestProject {
    /// Root path of the temporary project.
    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// Write a file at `rel_path` (directories created automatically).
    pub fn add_file(&self, rel_path: &str, content: &str) {
        let full = self.dir.path().join(rel_path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full, content).unwrap();
    }

    /// Open a fresh in-memory database with the BearWisdom schema.
    pub fn in_memory_db() -> Database {
        Database::open_in_memory().expect("failed to create in-memory database")
    }

    // -----------------------------------------------------------------
    // Fixture factories
    // -----------------------------------------------------------------

    /// C# project with models, an interface, a repository, and a service.
    pub fn csharp_service() -> Self {
        let p = Self { dir: TempDir::new().unwrap() };

        p.add_file("Models/Product.cs", r#"
namespace MyApp.Models
{
    public class Product
    {
        public int Id { get; set; }
        public string Name { get; set; }
        public decimal Price { get; set; }
    }
}
"#);

        p.add_file("Repositories/IProductRepository.cs", r#"
using MyApp.Models;

namespace MyApp.Repositories
{
    public interface IProductRepository
    {
        Product GetById(int id);
    }
}
"#);

        p.add_file("Repositories/ProductRepository.cs", r#"
using MyApp.Models;

namespace MyApp.Repositories
{
    public class ProductRepository : IProductRepository
    {
        public Product GetById(int id)
        {
            return new Product();
        }
    }
}
"#);

        p.add_file("Services/ProductService.cs", r#"
using MyApp.Models;
using MyApp.Repositories;

namespace MyApp.Services
{
    public class ProductService
    {
        private readonly IProductRepository _repository;

        public ProductService(IProductRepository repository)
        {
            _repository = repository;
        }

        public Product GetProduct(int id)
        {
            return _repository.GetById(id);
        }
    }
}
"#);

        p
    }

    /// Python project with inheritance and cross-module imports.
    pub fn python_app() -> Self {
        let p = Self { dir: TempDir::new().unwrap() };

        p.add_file("models.py", r#"
class Animal:
    def __init__(self, name: str):
        self.name = name

    def speak(self) -> str:
        raise NotImplementedError


class Dog(Animal):
    def speak(self) -> str:
        return f"{self.name} says Woof!"


class Cat(Animal):
    def speak(self) -> str:
        return f"{self.name} says Meow!"
"#);

        p.add_file("service.py", r#"
from models import Animal, Dog, Cat


def make_animal(kind: str, name: str) -> Animal:
    if kind == "dog":
        return Dog(name)
    return Cat(name)


def list_animals():
    return [
        make_animal("dog", "Rex"),
        make_animal("cat", "Whiskers"),
    ]
"#);

        p
    }

    /// TypeScript project with interfaces, classes, and exports.
    pub fn typescript_app() -> Self {
        let p = Self { dir: TempDir::new().unwrap() };

        p.add_file("types.ts", r#"
export interface User {
    id: number;
    name: string;
    email: string;
}

export interface CreateUserInput {
    name: string;
    email: string;
}
"#);

        p.add_file("user-service.ts", r#"
import { User, CreateUserInput } from './types';

export class UserService {
    private users: User[] = [];
    private nextId = 1;

    addUser(input: CreateUserInput): User {
        const user: User = { id: this.nextId++, ...input };
        this.users.push(user);
        return user;
    }

    findById(id: number): User | undefined {
        return this.users.find(u => u.id === id);
    }

    listUsers(): User[] {
        return [...this.users];
    }
}
"#);

        p
    }

    /// Multi-language project combining C#, Python, and TypeScript.
    pub fn multi_lang() -> Self {
        let p = Self { dir: TempDir::new().unwrap() };

        p.add_file("backend/Program.cs", r#"
namespace Backend
{
    public class AppConfig
    {
        public string ConnectionString { get; set; }
    }

    public class Startup
    {
        public void Configure(AppConfig config)
        {
        }
    }
}
"#);

        p.add_file("scripts/deploy.py", r#"
import os

def deploy(environment: str):
    print(f"Deploying to {environment}")

def rollback(version: str):
    print(f"Rolling back to {version}")
"#);

        p.add_file("frontend/app.ts", r#"
export class App {
    private name: string;

    constructor(name: string) {
        this.name = name;
    }

    start(): void {
        console.log(`Starting ${this.name}`);
    }
}
"#);

        p
    }

    /// Bazel / Starlark project fixture.
    ///
    /// Exercises:
    ///   - A rule implementation that uses `ctx.actions.run_shell`, `ctx.label.name`,
    ///     `ctx.label.package`, `ctx.outputs`, `ctx.file`, `ctx.attr`.
    ///   - An analysistest block using `env.expect.that_str` (3-level chain).
    ///   - A repository_rule using `repository_ctx.execute` and `repository_ctx.os.name`.
    ///   - A BUILD file with native rules (`cc_library`, `genrule`).
    pub fn starlark_bazel_project() -> Self {
        let p = Self { dir: TempDir::new().unwrap() };

        p.add_file("WORKSPACE", r#"workspace(name = "my_project")"#);

        p.add_file("tools/my_rule.bzl", r#"
def _my_rule_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run_shell(
        outputs = [out],
        inputs = ctx.files.srcs,
        command = "cat %s > %s" % (ctx.file.src.path, out.path),
    )
    return [DefaultInfo(files = depset([out]))]

my_rule = rule(
    implementation = _my_rule_impl,
    attrs = {
        "srcs": attr.label_list(allow_files = True),
        "src": attr.label(allow_single_file = True),
    },
)
"#);

        p.add_file("tools/my_rule_test.bzl", r#"
load("@bazel_skylib//lib:unittest.bzl", "analysistest", "asserts")

def _my_rule_test_impl(env):
    target = analysistest.target_under_test(env)
    env.expect.that_str(target.label.name).equals("expected")
    asserts.equals(env, "value", target[DefaultInfo].files.to_list()[0].basename)

my_rule_test = analysistest.make(
    _my_rule_test_impl,
)

def my_rule_test_suite(name):
    my_rule_test(
        name = name + "_test",
        target_under_test = ":my_target",
    )
"#);

        p.add_file("tools/fetch_tool.bzl", r#"
def _fetch_tool_impl(repository_ctx):
    result = repository_ctx.execute(["uname", "-s"])
    os_name = repository_ctx.os.name
    repository_ctx.file(
        "BUILD.bazel",
        content = "exports_files(['tool'])",
        executable = False,
    )

fetch_tool = repository_rule(
    implementation = _fetch_tool_impl,
    attrs = {
        "url": attr.string(),
    },
)
"#);

        p.add_file("BUILD.bazel", r#"
load("//tools:my_rule.bzl", "my_rule")

cc_library(
    name = "mylib",
    srcs = ["mylib.cc"],
    hdrs = ["mylib.h"],
)

genrule(
    name = "gen_header",
    srcs = [":mylib"],
    outs = ["gen.h"],
    cmd = "$(location :mylib) > $@",
)

my_rule(
    name = "my_target",
    srcs = [":mylib"],
)
"#);

        // Round 3: exercises env.expect.that_collection, that_int, that_bool chains
        // so the chain walker can produce starlark_ctx_chain edges for the env API.
        p.add_file("tools/analysistest_impl.bzl", r#"
load("@bazel_skylib//lib:unittest.bzl", "analysistest")

def _check_output_impl(ctx):
    env = analysistest.begin(ctx)
    target = analysistest.target_under_test(env)
    env.expect.that_collection(ctx.attr.srcs).contains("x")
    env.expect.that_collection(ctx.attr.srcs).is_empty()
    env.expect.that_int(len(target[DefaultInfo].files.to_list())).is_at_least(1)
    env.expect.that_bool(target[DefaultInfo].files.to_list() != []).is_true()
    env.assert_equals("foo", target.label.name)
    return analysistest.end(env)

check_output_test = analysistest.make(
    _check_output_impl,
)

def check_output_test_suite(name):
    check_output_test(
        name = name + "_test",
        target_under_test = ":my_target",
    )
"#);

        p
    }
}
