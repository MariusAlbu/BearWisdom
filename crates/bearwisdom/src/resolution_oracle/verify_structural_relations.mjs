// Independent TypeScript assignment diagnostics for the shared relation cohort.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";
const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3");
const cases = JSON.parse(readFileSync(process.argv[3] || new URL("./structural_relation_fixtures.json", import.meta.url), "utf8"));
let positives = 0, negatives = 0;
for (const test of cases) {
  const source = `export {}; ${test.preamble || ""}\ntype From = ${test.from}; type To = ${test.to};\ndeclare let from: From; const to: To = from;`;
  const fileName = resolve("bearwisdom-structural-relations.ts").replaceAll("\\", "/");
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
  const options = { strict: true, exactOptionalPropertyTypes: false, target: ts.ScriptTarget.ESNext, noEmit: true };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (path, ...args) => resolve(path).replaceAll("\\", "/") === fileName ? file : read(path, ...args);
  const program = ts.createProgram([fileName], options, host), checker = program.getTypeChecker();
  const diagnostics = ts.getPreEmitDiagnostics(program);
  assert.deepEqual(diagnostics.map(d => d.code), test.diagnosticCodes, test.name);
  const aliases = file.statements.filter(ts.isTypeAliasDeclaration);
  const from = aliases.find(d => d.name.text === "From"), to = aliases.find(d => d.name.text === "To");
  assert.equal(checker.isTypeAssignableTo(checker.getTypeFromTypeNode(from.type), checker.getTypeFromTypeNode(to.type)), test.assignable, test.name);
  if (test.assignable) { assert.equal(diagnostics.length, 0); positives++; }
  else {
    assert(diagnostics.length > 0); negatives++;
    for (const diagnostic of diagnostics) assert(diagnostic.file === file && diagnostic.start >= source.indexOf("const to:"), "negative must diagnose the assignment, not unrelated invalid source");
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, positives, diagnosticBackedNegatives: negatives }));
const obligations = JSON.parse(readFileSync(process.argv[4] || new URL("./structural_obligation_fixtures.json", import.meta.url), "utf8"));
for (const test of obligations) {
  const source = `export {}; ${test.preamble}\ntype From = ${test.from}; type To = ${test.to};`;
  const fileName = resolve("bearwisdom-structural-obligations.ts").replaceAll("\\", "/");
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
  const options = { strict: true, target: ts.ScriptTarget.ESNext, noEmit: true };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (path, ...args) => resolve(path).replaceAll("\\", "/") === fileName ? file : read(path, ...args);
  const program = ts.createProgram([fileName], options, host);
  const diagnostics = ts.getPreEmitDiagnostics(program);
  assert.deepEqual(diagnostics.map(d => d.code), test.diagnosticCodes, test.name);
  for (const diagnostic of diagnostics) assert(diagnostic.file === file && (test.sourceDiagnostic || diagnostic.start >= source.indexOf("type From")), test.name);
  if (test.proved || test.compilerAssignable !== undefined) {
    const checker = program.getTypeChecker();
    const aliases = file.statements.filter(ts.isTypeAliasDeclaration);
    const from = aliases.find(d => d.name.text === "From"), to = aliases.find(d => d.name.text === "To");
    assert.equal(checker.isTypeAssignableTo(checker.getTypeFromTypeNode(from.type), checker.getTypeFromTypeNode(to.type)), test.compilerAssignable ?? true, test.name);
  }
}
console.log(JSON.stringify({ compiler: ts.version, obligations: obligations.length, diagnosticBackedInvalidTypes: obligations.filter(t => t.diagnosticCodes.length).length,
  supportedProofs: obligations.filter(t => t.proved).length, legalAbstentions: obligations.filter(t => !t.proved && !t.diagnosticCodes.length).length }));
