// Independent compiler assignment evidence, including option-sensitive negatives.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";
const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3");
const cases = JSON.parse(readFileSync(new URL("./callable_relation_fixtures.json", import.meta.url), "utf8"));
let positives = 0, negatives = 0;
for (const test of cases) {
  const source = `export {}; ${test.preamble || ""} type From = ${test.from}; type To = ${test.to}; declare let from: From; const to: To = from;`;
  const fileName = resolve("bearwisdom-callable-relations.ts").replaceAll("\\", "/");
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
  const options = { strict: true, strictFunctionTypes: test.strictParameters ?? true,
    strictNullChecks: test.strictNulls ?? true, target: ts.ScriptTarget.ESNext, noEmit: true };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (path, ...args) => resolve(path).replaceAll("\\", "/") === fileName ? file : read(path, ...args);
  const program = ts.createProgram([fileName], options, host), checker = program.getTypeChecker();
  const diagnostics = ts.getPreEmitDiagnostics(program);
  assert.deepEqual(diagnostics.map(d => d.code), test.diagnosticCodes, test.name);
  const aliases = file.statements.filter(ts.isTypeAliasDeclaration);
  const from = aliases.find(d => d.name.text === "From"), to = aliases.find(d => d.name.text === "To");
  const a = checker.getTypeFromTypeNode(from.type), b = checker.getTypeFromTypeNode(to.type);
  assert.equal(checker.isTypeAssignableTo(a, b), test.assignable, test.name);
  assert.notEqual(a.getCallSignatures()[0].declaration, b.getCallSignatures()[0].declaration, test.name);
  for (const d of diagnostics) assert(d.file === file && (test.sourceDiagnostic || d.start >= source.indexOf("const to:")), test.name);
  if (test.sourceDiagnostic) { assert(diagnostics.length); continue; }
  if (test.assignable) positives++; else { assert(diagnostics.length); negatives++; }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, positives, diagnosticBackedNegatives: negatives,
  invalidSourceBarriers: cases.filter(t => t.sourceDiagnostic).length,
  legalAbstentions: cases.filter(t => t.supported === false && !t.sourceDiagnostic).length }));
