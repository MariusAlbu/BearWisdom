import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const cases = JSON.parse(readFileSync(new URL('local_initializer_fixtures.json', import.meta.url), 'utf8'));
let exactLocals = 0, exactConstructors = 0, exactCalls = 0;
for (const test of cases) for (const prefix of ['', 'export {}; ']) {
  const path = resolve('bearwisdom-local-initializer-virtual.ts').replaceAll('\\', '/');
  const text = prefix + test.source, options = { strict: true, noEmit: true, target: ts.ScriptTarget.ESNext, lib: ['lib.es5.d.ts'] };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (file, ...args) => file.replaceAll('\\', '/') === path ? ts.createSourceFile(path, text, options.target, true) : read(file, ...args);
  const program = ts.createProgram([path], options, host);
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b), [...test.diagnosticCodes].sort((a, b) => a - b), test.name);
  const source = program.getSourceFile(path), checker = program.getTypeChecker();
  if (test.checks) {
    const identifiers = []; const visit = node => { if (ts.isIdentifier(node)) identifiers.push(node); ts.forEachChild(node, visit); }; visit(source);
    for (const label of test.checks) {
      const at = text.indexOf(label.at) + label.at.length;
      const node = identifiers.find(n => n.getStart(source) === at && n.text === label.name); assert(node, test.name);
      if (label.expected === null) { assert(test.diagnosticCodes.length, test.name); continue; }
      const scope = node => { while (node && !ts.isFunctionDeclaration(node)) node = node.parent; return node; };
      let owner = scope(node), expected;
      while (owner && !expected) { expected = owner.parameters.find(p => p.name.getText(source) === label.expected); owner = scope(owner.parent); }
      assert(expected, `${test.name}: expected parameter`);
      const actual = checker.getTypeAtLocation(node), wanted = checker.getTypeAtLocation(expected.name);
      assert(actual === wanted, `${test.name}: ${checker.typeToString(actual)} != ${checker.typeToString(wanted)}`); exactLocals++;
    }
    continue;
  }
  if (test.diagnosticCodes.length) continue;
  let local, expected; const calls = [];
  const visit = node => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'excludeSet') local = node;
    if (ts.isParameter(node) && node.name.getText(source) === 'expected') expected = node;
    if (ts.isCallExpression(node)) calls.push(node);
    ts.forEachChild(node, visit);
  }; visit(source);
  assert(local && expected && ts.isNewExpression(local.initializer));
  const actual = checker.getTypeAtLocation(local.name), wanted = checker.getTypeAtLocation(expected.name);
  assert(actual === wanted, `${test.name}: ${checker.typeToString(actual)} != ${checker.typeToString(wanted)}`); exactLocals++;
  const constructor = checker.getResolvedSignature(local.initializer)?.declaration;
  assert(constructor && constructor.getSourceFile() === source);
  assert.equal(constructor.getStart(source), text.indexOf('new<U>'), test.name); exactConstructors++;
  assert.equal(calls.length, test.calls.length, test.name);
  for (const label of test.calls) {
    const call = calls.find(c => c.expression.getText(source) === label.expression); assert(call, label.expression);
    const selected = checker.getResolvedSignature(call)?.declaration; assert(selected && selected.getSourceFile() === source);
    assert.equal(selected.getStart(source), text.indexOf(label.declaration), `${test.name}: ${label.expression}`); exactCalls++;
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, scopeVariants: 2, exactLocals, exactConstructors, exactCalls }));
