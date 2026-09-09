// Read-only compiler validation of the independently authored multi-file cohort.
// Usage: node verify_modules.mjs <path-to-installed-typescript-module>
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";

const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3", "Review compiler adapter before changing its pin");
const cases = JSON.parse(readFileSync(process.argv[3] || new URL("./module_fixtures.json", import.meta.url), "utf8"));
assert(cases.length > 0, "No module fixtures found");
const normalize = path => resolve(path).replaceAll("\\", "/");
const root = normalize("bearwisdom-module-oracle-virtual");
let checked = 0;
for (const test of cases) {
  const refs = new Map(), declarations = new Map(), files = new Map();
  for (const file of test.files) {
    const path = normalize(root + "/" + file.path);
    let source = "", from = 0;
    for (const marker of file.source.matchAll(/\/\*@(\w+):(\d+)(?::\w+)?\*\//g)) {
      source += file.source.slice(from, marker.index);
      const map = marker[1] === "ref" ? refs : declarations;
      const id = Number(marker[2]);
      assert(!map.has(id), "Duplicate source marker " + id);
      map.set(id, { file: path, byte: Buffer.byteLength(source, "utf8") });
      from = marker.index + marker[0].length;
    }
    source += file.source.slice(from);
    assert(!files.has(path), "Duplicate fixture file");
    files.set(path, ts.createSourceFile(path, source, ts.ScriptTarget.Latest, true));
  }
  const options = { strict: true, allowJs: true, checkJs: true, target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext,
    moduleResolution: ts.ModuleResolutionKind.Bundler, lib: ["lib.esnext.d.ts"] };
  const configured = ts.convertCompilerOptionsFromJson(test.compilerOptions || {}, root);
  assert.deepEqual(configured.errors, [], test.name + " invalid compiler options");
  Object.assign(options, configured.options);
  assert([undefined, "signature"].includes(test.targetMode), "Unknown oracle target mode");
  const host = ts.createCompilerHost(options);
  const getSourceFile = host.getSourceFile.bind(host), fileExists = host.fileExists.bind(host);
  const directoryExists = host.directoryExists?.bind(host), readFile = host.readFile.bind(host);
  host.getCurrentDirectory = () => root;
  host.getSourceFile = (path, ...args) => files.get(normalize(path)) || getSourceFile(path, ...args);
  host.fileExists = path => files.has(normalize(path)) || fileExists(path);
  host.readFile = path => files.get(normalize(path))?.text ?? readFile(path);
  host.directoryExists = path => [...files.keys()].some(file => file.startsWith(normalize(path) + "/")) || !!directoryExists?.(path);
  const program = ts.createProgram([...files.keys()], options, host);
  if (Object.hasOwn(test, "diagnosticCodes")) {
    assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b),
      [...test.diagnosticCodes].sort((a, b) => a - b), test.name + " compiler diagnostics");
  }
  const checker = program.getTypeChecker();
  const site = node => ({ file: normalize(node.getSourceFile().fileName),
    byte: Buffer.byteLength(node.getSourceFile().text.slice(0, node.getStart()), "utf8") });
  const calls = new Map();
  function visit(node) {
    if (ts.isCallExpression(node)) {
      const callee = ts.isPropertyAccessExpression(node.expression) ? node.expression.name : node.expression;
      const key = JSON.stringify(site(callee));
      assert(!calls.has(key), "Ambiguous compiler call address");
      calls.set(key, { callee, call: node });
    }
    ts.forEachChild(node, visit);
  }
  for (const file of files.values()) visit(file);
  assert.equal(test.labels.length, refs.size, "Every reference needs a label");
  const labelled = new Set();
  for (const [reference, target] of test.labels) {
    assert(!labelled.has(reference), "Duplicate reference label");
    labelled.add(reference);
    const call = calls.get(JSON.stringify(refs.get(reference)));
    assert(call, "Compiler missed reference " + reference);
    const { callee } = call;
    let symbol = checker.getSymbolAtLocation(callee);
    // Import aliases navigate to the defining declaration, not the import token.
    if (symbol && (symbol.flags & ts.SymbolFlags.Alias)) symbol = checker.getAliasedSymbol(symbol);
    const signature = test.targetMode === "signature" ? checker.getResolvedSignature(call.call)?.declaration : undefined;
    const actual = test.targetMode === "signature" ? (signature ? [site(signature)] : []) : (symbol?.declarations || []).map(site);
    const expected = target === null ? [] : [declarations.get(target)];
    assert(target === null || expected[0], "Unknown declaration label");
    assert.deepEqual(actual, expected, test.name + " reference " + reference);
    checked++;
  }
}
console.log(JSON.stringify({ compiler: "TypeScript", version: ts.version, cases: cases.length, labelledCallsVerified: checked }));
