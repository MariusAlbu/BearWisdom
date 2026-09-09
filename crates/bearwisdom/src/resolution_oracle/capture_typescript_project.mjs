// Usage: node capture_typescript_project.mjs <typescript-module> <project-root>
//        <relative-tsconfig> <relative-source-prefix> <new-output-json>
// Compiler/source reads only; no emit, installs or project/index writes.
import { readFileSync, writeFileSync, realpathSync } from "node:fs";
import { createHash } from "node:crypto";
import { createRequire } from "node:module";
import { resolve, relative, isAbsolute, basename } from "node:path";
import { fileURLToPath } from "node:url";
import assert from "node:assert/strict";
import { captureCalls } from "./typescript_project_targets.mjs";

const normalize = path => resolve(path).replaceAll("\\", "/");
const digest = bytes => createHash("sha256").update(bytes).digest("hex");
const inside = (root, path) => { const r = relative(root, path); return r === "" || (!r.startsWith("..") && !isAbsolute(r)); };

export function capture(ts, projectRoot, configPath, sourcePrefix) {
  assert.equal(ts.version, "5.9.3", "Review adapter tests before changing the compiler pin");
  const root = normalize(realpathSync(projectRoot));
  const config = normalize(resolve(root, configPath)), selectedRoot = normalize(resolve(root, sourcePrefix));
  assert(inside(root, config) && inside(root, selectedRoot), "Configuration/cohort escapes project root");
  const inputs = new Map();
  const readFile = path => {
    try { const bytes = readFileSync(path); inputs.set(normalize(path), digest(bytes)); return bytes.toString("utf8"); }
    catch (error) { if (error.code === "ENOENT" || error.code === "ENOTDIR") return undefined; throw error; }
  };
  const errors = [];
  const parsed = ts.getParsedCommandLineOfConfigFile(config, {}, { ...ts.sys, readFile, onUnRecoverableConfigFileDiagnostic: d => errors.push(d) });
  assert(parsed && !errors.length, "Cannot load project configuration");
  const host = ts.createCompilerHost(parsed.options, true); host.readFile = readFile;
  host.writeFile = () => { throw new Error("Oracle must never emit compiler output"); };
  const program = ts.createProgram({ rootNames: parsed.fileNames, options: parsed.options, projectReferences: parsed.projectReferences, host });
  const diagnostics = [...parsed.errors, ...ts.getPreEmitDiagnostics(program)];
  const sources = [...program.getSourceFiles()].sort((a, b) => normalize(a.fileName).localeCompare(normalize(b.fileName), "en"));
  const files = new Map();
  for (const [index, sf] of sources.entries()) {
    const path = normalize(sf.fileName), bytes = readFileSync(path);
    assert.equal(bytes.toString("utf8"), sf.text, "Compiler text differs from indexed UTF-8 input: " + path);
    assert(Buffer.from(sf.text, "utf8").equals(bytes), "Non-UTF-8 source requires a separate coordinate adapter");
    const language = sf.scriptKind === ts.ScriptKind.TSX ? "tsx" : sf.scriptKind === ts.ScriptKind.JSX ? "jsx"
      : sf.scriptKind === ts.ScriptKind.JS ? "javascript" : sf.scriptKind === ts.ScriptKind.TS ? "typescript" : null;
    const id = index + 1;
    files.set(sf.fileName, { id, path, index_path: inside(root, path) ? relative(root, path).replaceAll("\\", "/") : `ext:oracle:${id}/${basename(path)}`,
      sha256: digest(bytes), language, selected: !!language && !sf.isDeclarationFile && inside(selectedRoot, path),
      source_scope: ts.isExternalModule(sf) ? "Module" : "Syntax" });
  }
  assert([...files.values()].some(f => f.selected), "Empty source cohort");
  const calls = captureCalls(ts, program, files);
  assert(calls.length, "Empty compiler call population");
  const manifest = {
    version: 2, compiler: { name: "TypeScript", version: ts.version }, root, config,
    selection: { kind: "all_call_expressions", source_prefix: sourcePrefix, split: "development", declaration_files: "support_only" },
    compiler_options: parsed.options,
    source_binding_order: program.getSourceFiles().map(sf => files.get(sf.fileName).id),
    files: [...files.values()], inputs: [...inputs].sort(([a], [b]) => a.localeCompare(b, "en")).map(([path, sha256]) => ({ path, sha256 })),
    calls, diagnostics: diagnostics.map(d => ({ code: d.code, category: ts.DiagnosticCategory[d.category], file: d.file ? normalize(d.file.fileName) : null,
      start: d.start ?? null, length: d.length ?? null, message: ts.flattenDiagnosticMessageText(d.messageText, "\n") })),
    limitations: ["development_cohort_not_held_out", "call_symbol_navigation_not_overload_dispatch", "missing_symbols_are_not_negative_labels",
      "recorded_input_hashes_not_complete_filesystem_resolution_fingerprint", "compiler_configuration_not_full_engine_configuration_parity"],
  };
  // Catch concurrent input changes during compiler analysis, before publishing.
  for (const input of [...manifest.inputs, ...manifest.files]) assert.equal(digest(readFileSync(input.path)), input.sha256, "Input changed during capture: " + input.path);
  return manifest;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assert.equal(process.argv.length, 7, "Expected compiler module, root, config, source prefix and output path");
  const ts = createRequire(import.meta.url)(process.argv[2]);
  const manifest = capture(ts, process.argv[3], process.argv[4], process.argv[5]);
  writeFileSync(process.argv[6], JSON.stringify(manifest, null, 2) + "\n", { flag: "wx" });
  const reasons = {};
  for (const call of manifest.calls) if (call.reason) reasons[call.reason] = (reasons[call.reason] || 0) + 1;
  console.log(JSON.stringify({ files: manifest.files.length, selected_files: manifest.files.filter(f => f.selected).length,
    compiler_calls: manifest.calls.length, labelled: manifest.calls.filter(c => c.target).length, unlabelled: reasons, diagnostics: manifest.diagnostics.length }));
}
