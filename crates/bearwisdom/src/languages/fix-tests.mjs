import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Get resolver name for a language
function getResolverName(lang) {
  const resolvePath = path.join(__dirname, lang, "resolve.rs");
  if (!fs.existsSync(resolvePath)) return null;
  const content = fs.readFileSync(resolvePath, "utf8");
  const m = content.match(/pub struct (\w+Resolver)/);
  return m ? m[1] : null;
}

// Fix extract_tests.rs
for (const lang of fs.readdirSync(__dirname)) {
  const testFile = path.join(__dirname, lang, "extract_tests.rs");
  if (!fs.existsSync(testFile)) continue;

  let c = fs.readFileSync(testFile, "utf8");

  // Replace use super::*; with specific imports
  // Need: extract function, ExtractedSymbol, ExtractedRef, EdgeKind, SymbolKind, Visibility
  const resolver = getResolverName(lang);

  let newImports = `use super::extract::extract;\nuse crate::types::{EdgeKind, ExtractedRef, ExtractedSymbol, SymbolKind, Visibility};`;

  // If tests use a Resolver, add those imports too
  if (resolver && c.includes(resolver)) {
    newImports += `\nuse super::resolve::${resolver};`;
    newImports += `\nuse crate::indexer::resolve::engine::{build_scope_chain, FileContext, LanguageResolver, RefContext, SymbolIndex, SymbolInfo};`;
  }
  if (c.includes("ProjectContext")) {
    newImports += `\nuse crate::indexer::project_context::ProjectContext;`;
  }
  if (c.includes("normalize_php_ns")) {
    newImports += `\nuse super::resolve::normalize_php_ns;`;
  }
  if (c.includes("HashMap")) {
    newImports += `\nuse std::collections::HashMap;`;
  }

  // Replace the first use super::*; line
  c = c.replace(/^\s*use super::\*;\n/m, newImports + "\n");

  // Also handle indented version (inside mod tests {})
  c = c.replace(
    /^(\s+)use super::\*;\n(\s+)use crate::types::\{[^}]+\};/m,
    `$1${newImports.split("\n").join("\n$1")}`,
  );

  // Handle special: c_lang extract takes two params
  if (lang === "c_lang") {
    // extract(source, "c") — already correct, extract is a fn taking (&str, &str)
  }

  // Handle Go's special re-exports (was pub(crate) use crate::types::...)
  // The extract.rs has a #[cfg(test)] pub(crate) use crate::types::...
  // Tests can access these through the module hierarchy

  fs.writeFileSync(testFile, c);
  console.log(`Fixed ${lang}/extract_tests.rs`);
}

// Fix resolve_tests.rs
for (const lang of fs.readdirSync(__dirname)) {
  const testFile = path.join(__dirname, lang, "resolve_tests.rs");
  if (!fs.existsSync(testFile)) continue;

  let c = fs.readFileSync(testFile, "utf8");
  const resolver = getResolverName(lang);

  let newImports = "";
  if (resolver) {
    newImports = `use super::resolve::${resolver};`;
  }
  newImports += `\nuse crate::indexer::project_context::ProjectContext;`;
  newImports += `\nuse crate::indexer::resolve::engine::{build_scope_chain, FileContext, LanguageResolver, RefContext, SymbolIndex, SymbolInfo};`;
  newImports += `\nuse crate::types::*;`;
  if (c.includes("HashMap")) {
    newImports += `\nuse std::collections::HashMap;`;
  }

  // Handle normalize_php_ns
  if (c.includes("normalize_php_ns")) {
    newImports += `\nuse super::resolve::normalize_php_ns;`;
  }

  c = c.replace(/^\s*use super::\*;\n/m, newImports + "\n");

  fs.writeFileSync(testFile, c);
  console.log(`Fixed ${lang}/resolve_tests.rs`);
}
