// Independent unique type/declaration-origin and invalid-owner controls.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";
const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3");
const cases = JSON.parse(readFileSync(new URL("./unique_symbol_fixtures.json", import.meta.url), "utf8"));
let declarations = 0, negatives = 0;
for (const test of cases) {
  const source = test.source, fileName = resolve("bearwisdom-unique-oracle.ts").replaceAll("\\", "/");
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
  const options = { noEmit: true, strict: true, target: ts.ScriptTarget.ESNext };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (path, ...args) => resolve(path).replaceAll("\\", "/") === fileName ? file : read(path, ...args);
  const program = ts.createProgram([fileName], options, host), checker = program.getTypeChecker();
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code), test.diagnostics, source);
  const owners = []; let annotations = 0;
  function visit(node) {
    if (ts.isTypeOperatorNode(node) && node.operator === ts.SyntaxKind.UniqueKeyword) {
      annotations++; const type = checker.getTypeFromTypeNode(node);
      if (type.flags & ts.TypeFlags.UniqueESSymbol) {
        assert.equal(type.symbol.declarations.length, 1, "merged property identities require a separate merge oracle");
        owners.push(Buffer.byteLength(source.slice(0, type.symbol.declarations[0].getStart(file))));
      } else { assert.equal(type.flags, ts.TypeFlags.ESSymbol); }
    }
    ts.forEachChild(node, visit);
  }
  visit(file); assert.equal(annotations, 1); assert.deepEqual(owners, test.ownerStarts, source);
  declarations += owners.length; negatives += test.diagnostics.length > 0 ? 1 : 0;
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, declarationOriginsVerified: declarations, diagnosticBackedNegatives: negatives }));
