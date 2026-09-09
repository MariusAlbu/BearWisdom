// Independent expression-type -> unique declaration source targets. No engine
// output is read, and no compiler files or dependency lockfiles are modified.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";
const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3");
const fixture = process.argv[3] || "./computed_unique_key_fixtures.json";
const cases = JSON.parse(readFileSync(new URL(fixture, import.meta.url), "utf8"));
const normalized = path => resolve(path).replaceAll("\\", "/");
let targets = 0, negativeKeys = 0, diagnosticCount = 0, queryTypes = 0;
for (const test of cases) {
  const files = new Map(test.files.map(f => [normalized(f.path), ts.createSourceFile(normalized(f.path), f.source, ts.ScriptTarget.Latest, true)]));
  const options = { noEmit: true, strict: true, target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext, moduleResolution: ts.ModuleResolutionKind.Node10 };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host), exists = host.fileExists.bind(host);
  host.getSourceFile = (path, ...args) => files.get(normalized(path)) || read(path, ...args);
  host.fileExists = path => files.has(normalized(path)) || exists(path);
  const program = ts.createProgram([...files.keys()], options, host), checker = program.getTypeChecker();
  const diagnostics = ts.getPreEmitDiagnostics(program);
  assert.deepEqual(diagnostics.map(d => d.code), test.diagnostics || [], JSON.stringify(test.files));
  diagnosticCount += diagnostics.length;
  const keys = [], queries = [], variables = [];
  for (const file of files.values()) {
    function visit(node) {
      if (ts.isComputedPropertyName(node)) keys.push({ file, node });
      if (ts.isTypeQueryNode(node)) queries.push({ file, node });
      if (ts.isVariableDeclaration(node)) variables.push({ file, node });
      ts.forEachChild(node, visit);
    }
    visit(file);
  }
  assert.equal(keys.length, test.keys.length);
  for (const expected of test.keys) {
    const file = files.get(normalized(expected.path));
    const { node } = keys.find(k => k.file === file && k.node.getText(file) === expected.expression) || {};
    assert(node, expected.expression);
    const type = checker.getTypeAtLocation(node.expression);
    if (expected.unbound) {
      assert(type.flags & (ts.TypeFlags.Any | ts.TypeFlags.Unknown), expected.expression);
      assert(diagnostics.some(d => d.file === file && d.start >= node.getStart(file) && d.start < node.getEnd()));
      negativeKeys++;
      continue;
    }
    assert(type.flags & ts.TypeFlags.UniqueESSymbol, expected.expression);
    assert.equal(type.symbol.declarations.length, 1, "merged unique declarations need compatibility proof");
    const owner = type.symbol.declarations[0], ownerFile = files.get(normalized(expected.ownerPath));
    assert.equal(owner.getSourceFile(), ownerFile);
    const start = ownerFile.text.indexOf(expected.owner); assert(start >= 0);
    assert.equal(Buffer.byteLength(ownerFile.text.slice(0, owner.getStart(ownerFile))), Buffer.byteLength(ownerFile.text.slice(0, start)));
    if (expected.unsupported) {
      let diagnosed = node;
      if (expected.diagnosticOwner) {
        assert(ts.isPropertyAccessExpression(node.expression), "invalid-type control must identify its receiver");
        const receiver = checker.getTypeAtLocation(node.expression.expression);
        const declarations = receiver.symbol?.declarations || [];
        assert.equal(declarations.length, 1);
        diagnosed = declarations[0]; assert(ts.isInterfaceDeclaration(diagnosed));
        assert.equal(diagnosed.getSourceFile(), file);
        assert.equal(diagnosed.getStart(file), file.text.indexOf(expected.diagnosticOwner));
      }
      assert(diagnostics.some(d => d.file === file && d.start >= diagnosed.getStart(file) && d.start < diagnosed.getEnd()));
      negativeKeys++;
    } else { targets++; }
  }
  if (test.queryTypes) assert.equal(queries.length, test.queryTypes.length);
  for (const expected of test.queryTypes || []) {
    const file = files.get(normalized(expected.path));
    const { node } = queries.find(q => q.file === file && q.node.getText(file) === expected.expression) || {};
    assert(node, expected.expression);
    const flags = ts.TypeFlags[expected.intrinsic]; assert(flags, expected.intrinsic);
    const type = checker.getTypeFromTypeNode(node);
    assert.equal(type.flags, flags);
    const binding = variables.find(v => v.file === file && v.node.name.getText(file) === expected.binding)?.node;
    assert(binding, expected.binding);
    assert.equal(checker.getTypeAtLocation(binding), type);
    queryTypes++;
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, exactUniqueDeclarationTargets: targets, diagnosticBackedNegativeKeys: negativeKeys, exactQueryTypes: queryTypes, diagnostics: diagnosticCount }));
