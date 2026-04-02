/**
 * Migrates all language extractors and resolvers into languages/<lang>/.
 * Run from: crates/bearwisdom/src/languages/
 *
 * This script:
 * 1. Copies files from parser/extractors/<lang>/ and indexer/resolve/rules/<lang>/
 * 2. Creates mod.rs with LanguagePlugin impl for each language
 * 3. Transforms extract.rs (strips old struct, fixes imports)
 * 4. Transforms resolve.rs (fixes super:: paths)
 * 5. Fixes test imports
 */

import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const SRC = path.resolve(__dirname, "..");
const EXTRACTORS = path.join(SRC, "parser", "extractors");
const RESOLVERS = path.join(SRC, "indexer", "resolve", "rules");
const LANGS_DIR = __dirname;

// Skip typescript (already migrated) and generic (special case, handle last)
const SKIP = new Set(["typescript"]);

// Language configs
const LANGUAGES = {
  bash: {
    id: "bash",
    lang_ids: ["shell"],
    extensions: [".sh", ".bash", ".zsh"],
    grammar: `Some(tree_sitter_bash::LANGUAGE.into())`,
    grammar_match: null, // single grammar, no match needed
    scope_kinds: "[]", // bash has no scope kinds
    resolver_dir: null,
    extractor_struct: null, // uses shared ExtractionResult
  },
  c_lang: {
    id: "c_lang",
    lang_ids: ["c", "cpp"],
    extensions: [".c", ".h", ".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx"],
    grammar: null, // needs match
    grammar_match: `match lang_id {
            "c" => Some(tree_sitter_c::LANGUAGE.into()),
            "cpp" => Some(tree_sitter_cpp::LANGUAGE.into()),
            _ => None,
        }`,
    scope_kinds: "extract::C_SCOPE_KINDS", // will use the one from extract.rs
    scope_comment: "// C/C++ share scope config but C++ has more (namespace, class)",
    resolver_dir: "c_lang",
    extractor_struct: null,
  },
  cpp: {
    id: "cpp",
    lang_ids: [], // c_lang handles cpp
    skip: true, // cpp is handled by c_lang
  },
  csharp: {
    id: "csharp",
    lang_ids: ["csharp"],
    extensions: [".cs"],
    grammar: `Some(tree_sitter_c_sharp::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "extract::CSHARP_SCOPE_KINDS",
    resolver_dir: "csharp",
    extractor_struct: "CSharpExtraction",
    has_routes: true,
  },
  dart: {
    id: "dart",
    lang_ids: ["dart"],
    extensions: [".dart"],
    grammar: `Some(tree_sitter_dart::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "extract::DART_SCOPE_KINDS",
    resolver_dir: "dart",
    extractor_struct: null,
  },
  elixir: {
    id: "elixir",
    lang_ids: ["elixir"],
    extensions: [".ex", ".exs"],
    grammar: `Some(tree_sitter_elixir::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "&[]",
    resolver_dir: "elixir",
    extractor_struct: null,
  },
  generic: {
    id: "generic",
    skip: true, // handled separately in mod.rs
  },
  go: {
    id: "go",
    lang_ids: ["go"],
    extensions: [".go"],
    grammar: `Some(tree_sitter_go::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "&[]", // Go uses qualified_prefix, not scope tree
    resolver_dir: "go",
    extractor_struct: "GoExtraction",
  },
  java: {
    id: "java",
    lang_ids: ["java"],
    extensions: [".java"],
    grammar: `Some(tree_sitter_java::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "extract::JAVA_SCOPE_KINDS",
    resolver_dir: "java",
    extractor_struct: "JavaExtraction",
  },
  javascript: {
    id: "javascript",
    lang_ids: ["javascript", "jsx"],
    extensions: [".js", ".jsx", ".mjs", ".cjs"],
    grammar: `Some(tree_sitter_javascript::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "&[]", // JS scope kinds are inside extract fn
    resolver_dir: null, // TS resolver handles JS
    extractor_struct: null,
  },
  kotlin: {
    id: "kotlin",
    lang_ids: ["kotlin"],
    extensions: [".kt", ".kts"],
    grammar: `Some(tree_sitter_kotlin_ng::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "extract::KOTLIN_SCOPE_KINDS",
    resolver_dir: "kotlin",
    extractor_struct: null,
  },
  php: {
    id: "php",
    lang_ids: ["php"],
    extensions: [".php"],
    grammar: `Some(tree_sitter_php::LANGUAGE_PHP.into())`,
    grammar_match: null,
    scope_kinds: "extract::PHP_SCOPE_KINDS",
    resolver_dir: "php",
    extractor_struct: null,
  },
  python: {
    id: "python",
    lang_ids: ["python"],
    extensions: [".py", ".pyi"],
    grammar: `Some(tree_sitter_python::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "extract::PYTHON_SCOPE_KINDS",
    resolver_dir: "python",
    extractor_struct: "PythonExtraction",
  },
  ruby: {
    id: "ruby",
    lang_ids: ["ruby"],
    extensions: [".rb", ".rake", ".gemspec"],
    grammar: `Some(tree_sitter_ruby::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "extract::RUBY_SCOPE_KINDS",
    resolver_dir: "ruby",
    extractor_struct: null,
  },
  rust: {
    id: "rust_lang",
    lang_ids: ["rust"],
    extensions: [".rs"],
    grammar: `Some(tree_sitter_rust::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "&[]", // Rust uses qualified_prefix, not scope tree
    resolver_dir: "rust_lang",
    extractor_struct: "RustExtraction",
    extractor_dir: "rust", // extractor dir name differs from resolver
  },
  scala: {
    id: "scala",
    lang_ids: ["scala"],
    extensions: [".scala", ".sc"],
    grammar: `Some(tree_sitter_scala::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "extract::SCALA_SCOPE_KINDS",
    resolver_dir: "scala",
    extractor_struct: null,
  },
  swift: {
    id: "swift",
    lang_ids: ["swift"],
    extensions: [".swift"],
    grammar: `Some(tree_sitter_swift::LANGUAGE.into())`,
    grammar_match: null,
    scope_kinds: "extract::SWIFT_SCOPE_KINDS",
    resolver_dir: "swift",
    extractor_struct: null,
  },
};

function copyFiles(srcDir, destDir, excludeFiles = []) {
  if (!fs.existsSync(srcDir)) return [];
  const files = fs.readdirSync(srcDir).filter((f) => !excludeFiles.includes(f));
  fs.mkdirSync(destDir, { recursive: true });
  const copied = [];
  for (const f of files) {
    const src = path.join(srcDir, f);
    const dest = path.join(destDir, f);
    if (fs.statSync(src).isFile()) {
      fs.copyFileSync(src, dest);
      copied.push(f);
    }
  }
  return copied;
}

function fixFile(filePath, replacements) {
  if (!fs.existsSync(filePath)) return;
  let content = fs.readFileSync(filePath, "utf8");
  for (const [from, to] of replacements) {
    if (typeof from === "string") {
      content = content.split(from).join(to);
    } else {
      content = content.replace(from, to);
    }
  }
  fs.writeFileSync(filePath, content);
}

function generateModRs(config) {
  const hasResolver = !!config.resolver_dir;
  const langDir = config.extractor_dir || config.id;
  const structName =
    config.id.charAt(0).toUpperCase() +
    config.id
      .slice(1)
      .replace(/_(\w)/g, (_, c) => c.toUpperCase()) +
    "Plugin";
  const resolverName = structName.replace("Plugin", "Resolver");

  // Build extractor sub-module list from files
  const targetDir = path.join(LANGS_DIR, langDir === "rust" ? "rust_lang" : langDir);
  const files = fs.existsSync(targetDir) ? fs.readdirSync(targetDir) : [];
  const subModules = files
    .filter(
      (f) =>
        f.endsWith(".rs") &&
        f !== "mod.rs" &&
        f !== "extract.rs" &&
        f !== "resolve.rs" &&
        !f.includes("test"),
    )
    .map((f) => f.replace(".rs", ""))
    .sort();

  // Determine which are resolver-related
  const resolverMods = ["chain", "builtins"];
  const extractorMods = subModules.filter((m) => !resolverMods.includes(m));
  const resolverSubMods = subModules.filter((m) => resolverMods.includes(m));

  const langIdsStr = config.lang_ids.map((id) => `"${id}"`).join(", ");
  const extsStr = config.extensions.map((e) => `"${e}"`).join(", ");

  let grammarBody;
  if (config.grammar_match) {
    grammarBody = config.grammar_match;
  } else {
    // Single grammar for all IDs
    grammarBody = config.grammar;
  }

  let lines = [];
  lines.push(`//! ${config.id} language plugin.\n`);

  // Extractor sub-modules
  for (const m of extractorMods) {
    const vis = m === "decorators" ? "pub(crate) " : "";
    lines.push(`${vis}mod ${m};`);
  }
  lines.push(`pub mod extract;`);
  lines.push("");

  // Resolver sub-modules
  if (hasResolver) {
    for (const m of resolverSubMods) {
      lines.push(`mod ${m};`);
    }
    lines.push(`pub mod resolve;`);
    lines.push("");
  }

  // Tests
  const hasExtractTests = files.includes("extract_tests.rs");
  const hasResolveTests = files.includes("resolve_tests.rs");
  if (hasExtractTests) {
    lines.push(`#[cfg(test)]`);
    lines.push(`#[path = "extract_tests.rs"]`);
    lines.push(`mod extract_tests;`);
    lines.push("");
  }
  if (hasResolveTests) {
    lines.push(`#[cfg(test)]`);
    lines.push(`#[path = "resolve_tests.rs"]`);
    lines.push(`mod resolve_tests;`);
    lines.push("");
  }

  lines.push(`use crate::languages::LanguagePlugin;`);
  lines.push(`use crate::parser::extractors::ExtractionResult;`);
  lines.push(`use crate::parser::scope_tree::ScopeKind;`);
  lines.push("");

  if (hasResolver) {
    lines.push(`pub use resolve::${resolverName};`);
    lines.push("");
  }

  lines.push(`pub struct ${structName};`);
  lines.push("");
  lines.push(`impl LanguagePlugin for ${structName} {`);
  lines.push(`    fn id(&self) -> &str { "${config.id}" }`);
  lines.push("");
  lines.push(`    fn language_ids(&self) -> &[&str] { &[${langIdsStr}] }`);
  lines.push("");
  lines.push(`    fn extensions(&self) -> &[&str] { &[${extsStr}] }`);
  lines.push("");

  // grammar()
  lines.push(`    fn grammar(&self, lang_id: &str) -> Option<tree_sitter::Language> {`);
  if (config.grammar_match) {
    lines.push(`        ${config.grammar_match}`);
  } else {
    lines.push(`        let _ = lang_id;`);
    lines.push(`        ${config.grammar}`);
  }
  lines.push(`    }`);
  lines.push("");

  // scope_kinds()
  lines.push(`    fn scope_kinds(&self) -> &[ScopeKind] { ${config.scope_kinds} }`);
  lines.push("");

  // extract()
  lines.push(
    `    fn extract(&self, source: &str, file_path: &str, lang_id: &str) -> ExtractionResult {`,
  );
  if (config.id === "c_lang") {
    lines.push(
      `        extract::extract(source, lang_id)`,
    );
  } else {
    lines.push(`        let _ = (file_path, lang_id);`);
    lines.push(`        extract::extract(source)`);
  }
  lines.push(`    }`);

  lines.push(`}`);

  return lines.join("\n");
}

// Main migration
let count = 0;
for (const [key, config] of Object.entries(LANGUAGES)) {
  if (config.skip || SKIP.has(key)) continue;

  const extractorDir = config.extractor_dir || key;
  const targetDirName = config.id === "rust_lang" ? "rust_lang" : key;
  const targetDir = path.join(LANGS_DIR, targetDirName);
  const srcExtractor = path.join(EXTRACTORS, extractorDir);
  const srcResolver = config.resolver_dir
    ? path.join(RESOLVERS, config.resolver_dir)
    : null;

  console.log(`\n=== Migrating ${key} → languages/${targetDirName}/ ===`);

  // 1. Copy extractor files
  fs.mkdirSync(targetDir, { recursive: true });
  const extractorFiles = copyFiles(srcExtractor, targetDir, ["mod.rs"]);
  // Copy mod.rs as extract.rs
  if (fs.existsSync(path.join(srcExtractor, "mod.rs"))) {
    fs.copyFileSync(
      path.join(srcExtractor, "mod.rs"),
      path.join(targetDir, "extract.rs"),
    );
    extractorFiles.push("extract.rs");
  }
  console.log(`  Extractor: ${extractorFiles.join(", ")}`);

  // 2. Copy resolver files
  if (srcResolver && fs.existsSync(srcResolver)) {
    const resolverFiles = copyFiles(srcResolver, targetDir, ["mod.rs"]);
    if (fs.existsSync(path.join(srcResolver, "mod.rs"))) {
      fs.copyFileSync(
        path.join(srcResolver, "mod.rs"),
        path.join(targetDir, "resolve.rs"),
      );
      resolverFiles.push("resolve.rs");
    }
    console.log(`  Resolver: ${resolverFiles.join(", ")}`);
  }

  // 3. Rename tests.rs to extract_tests.rs (avoid conflict with resolve tests)
  if (fs.existsSync(path.join(targetDir, "tests.rs"))) {
    fs.renameSync(
      path.join(targetDir, "tests.rs"),
      path.join(targetDir, "extract_tests.rs"),
    );
  }
  // Rename resolver tests if they exist
  // (they were copied directly, no rename needed — but check for conflict)

  // 4. Fix extract.rs
  const extractPath = path.join(targetDir, "extract.rs");
  if (fs.existsSync(extractPath)) {
    fixFile(extractPath, [
      // Remove old sub-module declarations (they move to mod.rs)
      [/^mod \w+;\n/gm, ""],
      [/^pub\(super\) mod \w+;\n/gm, ""],
      // Add super:: imports for sub-modules at the top
      // Remove #[cfg(test)] mod tests block at the end
      [/\n#\[cfg\(test\)\]\n#\[path = "tests\.rs"\]\nmod tests;\n?/g, ""],
      [/\n#\[cfg\(test\)\]\nmod tests;\n?/g, ""],
      // Fix emit_chain_type_ref path
      [
        "super::emit_chain_type_ref",
        "crate::parser::extractors::emit_chain_type_ref",
      ],
      [
        "super::super::emit_chain_type_ref",
        "crate::parser::extractors::emit_chain_type_ref",
      ],
    ]);

    // Replace per-language extraction struct with ExtractionResult
    if (config.extractor_struct) {
      const struct_name = config.extractor_struct;
      let content = fs.readFileSync(extractPath, "utf8");
      // Remove struct definition
      content = content.replace(
        new RegExp(
          `pub struct ${struct_name} \\{[\\s\\S]*?\\}\n*`,
        ),
        "",
      );
      // Replace constructor calls
      content = content.replace(
        new RegExp(`${struct_name} \\{\\s*symbols,\\s*refs,\\s*has_errors\\s*\\}`, "g"),
        "ExtractionResult::new(symbols, refs, has_errors)",
      );
      // For CSharp which has routes and db_sets
      if (config.has_routes) {
        content = content.replace(
          new RegExp(
            `${struct_name} \\{\\s*symbols,\\s*refs,\\s*routes,\\s*db_sets,\\s*has_errors\\s*\\}`,
            "g",
          ),
          "ExtractionResult::with_connectors(symbols, refs, routes, db_sets, has_errors)",
        );
      }
      // Replace empty error returns
      content = content.replace(
        new RegExp(
          `${struct_name} \\{[\\s\\S]*?has_errors: true,?\\s*\\}`,
          "g",
        ),
        `ExtractionResult { symbols: vec![], refs: vec![], routes: vec![], db_sets: vec![], has_errors: true }`,
      );
      // Add ExtractionResult import if not present
      if (!content.includes("ExtractionResult")) {
        content = content.replace(
          "use crate::parser::scope_tree",
          "use crate::parser::extractors::ExtractionResult;\nuse crate::parser::scope_tree",
        );
      }
      // Fix return type
      content = content.replace(
        new RegExp(`-> ${struct_name}`, "g"),
        "-> ExtractionResult",
      );
      fs.writeFileSync(extractPath, content);
    }

    // Ensure ExtractionResult import exists
    let content = fs.readFileSync(extractPath, "utf8");
    if (
      !content.includes("ExtractionResult") &&
      !content.includes("super::ExtractionResult")
    ) {
      // Add import
      const firstUse = content.indexOf("use ");
      if (firstUse >= 0) {
        content =
          content.slice(0, firstUse) +
          "use crate::parser::extractors::ExtractionResult;\n" +
          content.slice(firstUse);
        fs.writeFileSync(extractPath, content);
      }
    }

    // Make scope kinds public for the plugin to reference
    let c2 = fs.readFileSync(extractPath, "utf8");
    c2 = c2.replace(
      /static (\w+_SCOPE_KINDS)/g,
      "pub(crate) static $1",
    );
    fs.writeFileSync(extractPath, c2);
  }

  // 5. Fix resolve.rs
  const resolvePath = path.join(targetDir, "resolve.rs");
  if (fs.existsSync(resolvePath)) {
    fixFile(resolvePath, [
      // Remove old sub-module declarations
      [/^mod \w+;\n/gm, ""],
      // Remove test module
      [/\n#\[cfg\(test\)\]\n#\[path = "tests\.rs"\]\nmod tests;\n?/g, ""],
      [/\n#\[cfg\(test\)\]\nmod tests;\n?/g, ""],
      // Fix engine path (various depths)
      [
        "use super::super::super::engine::",
        "use crate::indexer::resolve::engine::",
      ],
      [
        "use super::super::engine::",
        "use crate::indexer::resolve::engine::",
      ],
      // Fix type_env path
      [
        "use super::super::super::type_env::",
        "use crate::indexer::resolve::type_env::",
      ],
      [
        "use super::super::type_env::",
        "use crate::indexer::resolve::type_env::",
      ],
      // Fix builtins import (was mod builtins; now use super::builtins)
      // After removing mod declarations, add use super:: for builtins and chain
    ]);

    // Add use super:: for builtins/chain if they exist
    let rc = fs.readFileSync(resolvePath, "utf8");
    const hasBuiltins = fs.existsSync(path.join(targetDir, "builtins.rs"));
    const hasChain = fs.existsSync(path.join(targetDir, "chain.rs"));
    if (hasBuiltins || hasChain) {
      const parts = [];
      if (hasBuiltins) parts.push("builtins");
      if (hasChain) parts.push("chain");
      // Check if already imported
      if (!rc.includes("use super::{") && !rc.includes("use super::builtins")) {
        // Add after the first use crate:: line
        const insertPoint = rc.indexOf("use crate::");
        if (insertPoint >= 0) {
          const lineEnd = rc.indexOf("\n", insertPoint);
          rc =
            rc.slice(0, lineEnd + 1) +
            `use super::{${parts.join(", ")}};\n` +
            rc.slice(lineEnd + 1);
        }
      }
    }
    fs.writeFileSync(resolvePath, rc);
  }

  // 6. Fix chain.rs
  const chainPath = path.join(targetDir, "chain.rs");
  if (fs.existsSync(chainPath)) {
    fixFile(chainPath, [
      [
        "use super::super::super::engine::",
        "use crate::indexer::resolve::engine::",
      ],
      [
        "use super::super::engine::",
        "use crate::indexer::resolve::engine::",
      ],
      [
        "use super::super::super::type_env::",
        "use crate::indexer::resolve::type_env::",
      ],
      [
        "use super::super::type_env::",
        "use crate::indexer::resolve::type_env::",
      ],
    ]);
  }

  // 7. Fix calls.rs (emit_chain_type_ref)
  const callsPath = path.join(targetDir, "calls.rs");
  if (fs.existsSync(callsPath)) {
    fixFile(callsPath, [
      [
        "super::emit_chain_type_ref",
        "crate::parser::extractors::emit_chain_type_ref",
      ],
      [
        "super::super::emit_chain_type_ref",
        "crate::parser::extractors::emit_chain_type_ref",
      ],
    ]);
  }

  // 8. Fix test files
  const extractTestPath = path.join(targetDir, "extract_tests.rs");
  if (fs.existsSync(extractTestPath)) {
    let tc = fs.readFileSync(extractTestPath, "utf8");
    // Replace use super::* with specific imports
    if (tc.includes("use super::*;")) {
      tc = tc.replace(
        "use super::*;",
        "use super::extract::extract;\nuse crate::types::{ExtractedRef, ExtractedSymbol};",
      );
    }
    fs.writeFileSync(extractTestPath, tc);
  }

  const resolveTestPath = path.join(targetDir, "resolve_tests.rs");
  if (fs.existsSync(resolveTestPath)) {
    let tc = fs.readFileSync(resolveTestPath, "utf8");
    tc = tc.replace("use super::*;", "use super::resolve::*;\nuse crate::indexer::resolve::engine::{LanguageResolver, RefContext};");
    fs.writeFileSync(resolveTestPath, tc);
  }

  // 9. Generate mod.rs
  const modContent = generateModRs(config);
  fs.writeFileSync(path.join(targetDir, "mod.rs"), modContent);
  console.log(`  Generated mod.rs with ${config.id} plugin`);

  count++;
}

console.log(`\n=== Migrated ${count} languages ===`);
