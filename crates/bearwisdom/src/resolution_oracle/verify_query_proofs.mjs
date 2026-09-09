// Independent compiler diagnostics and unique-symbol origins for private query staging.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";
const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3");
const cases = JSON.parse(readFileSync(new URL("./query_proof_fixtures.json", import.meta.url), "utf8"));
let exactOrigins = 0;
for (const test of cases) {
  const path = resolve("bearwisdom-query-proof-virtual.ts").replaceAll("\\", "/");
  const options = { strict: true, noEmit: true, target: ts.ScriptTarget.ESNext };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (file, ...args) => file.replaceAll("\\", "/") === path
    ? ts.createSourceFile(path, test.source, options.target, true) : read(file, ...args);
  const program = ts.createProgram([path], options, host);
  const diagnostics = ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b);
  assert.deepEqual(diagnostics, [...test.diagnosticCodes].sort((a, b) => a - b), test.name);
  if (!test.admitted) continue;
  const source = program.getSourceFile(path), checker = program.getTypeChecker();
  const matches = [];
  const visit = node => {
    if (ts.isComputedPropertyName(node) && node.getText(source) === test.key) matches.push(node);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert(matches.length > 0, test.name);
  for (const match of matches) {
    const type = checker.getTypeAtLocation(match.expression);
    assert(type.flags & ts.TypeFlags.UniqueESSymbol, test.name);
    assert.equal(type.symbol.declarations.length, 1, test.name);
    assert.equal(type.symbol.declarations[0].name.getText(source), test.origin, test.name);
    exactOrigins++;
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, exactOrigins, diagnosticBackedNegatives: cases.filter(test => !test.admitted).length }));
