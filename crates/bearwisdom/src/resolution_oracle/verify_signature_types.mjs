// Read-only independent compiler binding checks for source-owned type parameters.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";

const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3");
const cases = JSON.parse(readFileSync(new URL("./signature_type_fixtures.json", import.meta.url), "utf8"));
let checked = 0;
for (const test of cases) {
  const fileName = resolve("bearwisdom-signature-oracle.ts").replaceAll("\\", "/"), source = test.source;
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
  const refs = new Map(), declarations = new Map();
  for (const marker of source.matchAll(/\/\*@(ref|decl):(\d+)\*\//g)) {
    const map = marker[1] === "ref" ? refs : declarations, id = Number(marker[2]);
    assert(!map.has(id));
    map.set(id, marker.index + marker[0].length);
  }
  const options = { strict: true, target: ts.ScriptTarget.ESNext, noEmit: true };
  const host = ts.createCompilerHost(options), getSourceFile = host.getSourceFile.bind(host);
  host.getSourceFile = (path, ...args) => resolve(path).replaceAll("\\", "/") === fileName ? file : getSourceFile(path, ...args);
  const program = ts.createProgram([fileName], options, host), checker = program.getTypeChecker();
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a,b) => a-b), test.diagnosticCodes, test.name);
  const identifiers = new Map();
  function visit(node) { if (ts.isIdentifier(node)) identifiers.set(node.getStart(file), node); ts.forEachChild(node, visit); }
  visit(file);
  assert.equal(refs.size, test.labels.length);
  const seen = new Set();
  for (const [reference, target] of test.labels) {
    assert(!seen.has(reference)); seen.add(reference);
    const node = identifiers.get(refs.get(reference)); assert(node);
    const symbol = checker.getSymbolAtLocation(node);
    const actual = (symbol?.declarations || []).map(d => {
      assert(ts.isTypeParameterDeclaration(d), "Expected a type parameter declaration");
      return Buffer.byteLength(source.slice(0, d.getStart(file)), "utf8");
    });
    assert(target === null || declarations.has(target));
    const expected = target === null ? [] : [Buffer.byteLength(source.slice(0, declarations.get(target)), "utf8")];
    assert.deepEqual(actual, expected, test.name + " reference " + reference); checked++;
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, typeBindingsVerified: checked }));
