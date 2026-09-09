// Independently validate the explicit labels in scope_tests.rs with TypeScript.
// Usage: node crates/bearwisdom/src/resolution_oracle/verify_typescript.mjs <path-to-typescript-module>
// Read-only: no installs, source changes, index writes or automatic rebaselining.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import assert from "node:assert/strict";

const require = createRequire(import.meta.url);
const ts = require(process.argv[2] || "typescript");
const sourcePath = new URL("./scope_tests.rs", import.meta.url);
const tests = readFileSync(sourcePath, "utf8");
const classes = JSON.parse(tests.match(/const CLASSES: &str = ("(?:[^"\\]|\\.)*")/)[1]);
const cases = [...tests.matchAll(/check\(\s*("(?:[^"\\]|\\.)*"),\s*&\[([\s\S]*?)\]\s*,?\s*\);/g)];
assert(cases.length >= 6, "No complete fixture corpus found; update the fixture reader explicitly");
assert.equal(cases.length, (tests.match(/\n\s*check\(/g) || []).length, "A fixture was skipped by the reader");
let checked = 0;
for (const [index, match] of cases.entries()) {
  const marked = classes + JSON.parse(match[1]);
  const refs = new Map(), declarations = new Map();
  let source = "", from = 0;
  for (const marker of marked.matchAll(/\/\*@(\w+):(\d+)(?::\w+)?\*\//g)) {
    source += marked.slice(from, marker.index);
    const byte = Buffer.byteLength(source, "utf8");
    (marker[1] === "ref" ? refs : declarations).set(Number(marker[2]), byte);
    from = marker.index + marker[0].length;
  }
  source += marked.slice(from);
  const sf = ts.createSourceFile("fixture.ts", source, ts.ScriptTarget.Latest, true);
  // Real compiler standard-library declarations are needed for inference over
  // arrays, promises and other built-in structural types. No network/install.
  const options = { strict: true, target: ts.ScriptTarget.ESNext, lib: ["lib.esnext.d.ts"] };
  const host = ts.createCompilerHost(options);
  const getSourceFile = host.getSourceFile.bind(host);
  host.getSourceFile = (path, ...args) => path === "fixture.ts" ? sf : getSourceFile(path, ...args);
  const program = ts.createProgram(["fixture.ts"], options, host);
  const checker = program.getTypeChecker();
  const calls = new Map();
  const byteAt = node => Buffer.byteLength(source.slice(0, node.getStart(sf)), "utf8");
  function visit(node) {
    if (ts.isCallExpression(node)) {
      const callee = node.expression;
      calls.set(byteAt(ts.isPropertyAccessExpression(callee) ? callee.name : callee), node);
    }
    ts.forEachChild(node, visit);
  }
  visit(sf);
  const labels = [...match[2].matchAll(/\((\d+),\s*(?:Some\((\d+)\)|(None))\)/g)];
  assert.equal(labels.length, refs.size, "Each marked reference needs an explicit label");
  for (const label of labels) {
    const reference = Number(label[1]);
    const call = calls.get(refs.get(reference));
    assert(call, "Compiler did not extract labelled call " + reference);
    const expression = call.expression;
    const symbol = checker.getSymbolAtLocation(ts.isPropertyAccessExpression(expression) ? expression.name : expression);
    const actual = (symbol?.declarations || []).map(declaration => {
      const owner = declaration.getSourceFile();
      // An external declaration must never collide with a fixture byte offset.
      return owner === sf ? byteAt(declaration) : { file: owner.fileName, start: declaration.getStart(owner) };
    });
    const expected = label[2] === undefined ? [] : [declarations.get(Number(label[2]))];
    assert.deepEqual(actual, expected, "Compiler disagreement at case " + index + " reference " + reference);
    checked++;
  }
}
console.log(JSON.stringify({ compiler: "TypeScript", version: ts.version, cases: cases.length, labelledCallsVerified: checked }));
