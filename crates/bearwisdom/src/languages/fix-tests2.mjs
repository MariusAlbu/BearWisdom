import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// Deduplicate use statements in a Rust file without removing necessary duplicate lines like }
function deduplicateUseStatements(content) {
  const lines = content.split("\n");
  const seen = new Set();
  const result = [];

  for (const line of lines) {
    const trimmed = line.trim();
    // Only deduplicate `use` statements
    if (trimmed.startsWith("use ") && trimmed.endsWith(";")) {
      if (seen.has(trimmed)) continue;
      seen.add(trimmed);
    }
    result.push(line);
  }
  return result.join("\n");
}

// Process all test files
for (const lang of fs.readdirSync(__dirname)) {
  const langDir = path.join(__dirname, lang);
  if (!fs.statSync(langDir).isDirectory()) continue;

  for (const testFile of ["extract_tests.rs", "resolve_tests.rs"]) {
    const filePath = path.join(langDir, testFile);
    if (!fs.existsSync(filePath)) continue;

    let c = fs.readFileSync(filePath, "utf8");
    c = deduplicateUseStatements(c);
    fs.writeFileSync(filePath, c);
  }

  // Fix decorators.rs inline tests
  const decoPath = path.join(langDir, "decorators.rs");
  if (fs.existsSync(decoPath)) {
    let c = fs.readFileSync(decoPath, "utf8");
    // Fix: use super::super::extract → use super::super::extract::extract
    if (
      c.includes("use super::super::extract;") &&
      !c.includes("use super::super::extract::extract;")
    ) {
      c = c.replace(
        "use super::super::extract;",
        `use super::super::extract::extract;`,
      );
    }
    fs.writeFileSync(decoPath, c);
  }
}

console.log("Deduplicated and fixed all test files");
