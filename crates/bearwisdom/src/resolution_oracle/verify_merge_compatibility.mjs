// Compiler diagnostic evidence, independently maintained from merge admission.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";
const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3");
const cases = JSON.parse(readFileSync(process.argv[3] || new URL("./merge_compatibility_fixtures.json", import.meta.url), "utf8"));
let positives = 0, negatives = 0, unsupported = 0;
for (const test of cases) {
  const path = resolve("bearwisdom-merge-virtual.ts").replaceAll("\\", "/");
  const options = { strict: true, noEmit: true, target: ts.ScriptTarget.ESNext };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (file, ...args) => file.replaceAll("\\", "/") === path
    ? ts.createSourceFile(path, test.source, options.target, true) : read(file, ...args);
  const program = ts.createProgram([path], options, host);
  const diagnostics = ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b);
  assert.deepEqual(diagnostics, [...test.diagnosticCodes].sort((a, b) => a - b), test.name);
  if (test.unsupported) { assert.equal(test.admitted, false); assert.equal(diagnostics.length, 0); unsupported++; }
  else if (test.admitted) { assert.equal(diagnostics.length, 0); positives++; }
  else { assert(diagnostics.length > 0, test.name); negatives++; }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, positives, diagnosticBackedNegatives: negatives, legalUnsupportedCases: unsupported }));
