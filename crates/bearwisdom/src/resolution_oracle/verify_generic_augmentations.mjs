// Pinned compiler admission diagnostics and exact source call declarations.
// Both script and module scopes are checked, independently of engine outputs.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const cases = JSON.parse(readFileSync(new URL('generic_augmentation_fixtures.json', import.meta.url), 'utf8'));
let exactCallTargets = 0;
for (const test of cases) {
  for (const prefix of ['', 'export {}; ']) {
    const path = resolve('bearwisdom-generics-virtual.ts').replaceAll('\\', '/');
    const text = prefix + test.source;
    const options = { strict: true, noEmit: true, target: ts.ScriptTarget.ESNext };
    const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
    host.getSourceFile = (file, ...args) => file.replaceAll('\\', '/') === path
      ? ts.createSourceFile(path, text, options.target, true) : read(file, ...args);
    const program = ts.createProgram([path], options, host);
    const diagnostics = ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b);
    assert.deepEqual(diagnostics, [...test.diagnosticCodes].sort((a, b) => a - b), `${test.name} (${prefix})`);
    if (test.admitted || test.unsupported) assert.equal(diagnostics.length, 0, test.name);
    else assert(diagnostics.length, test.name);
    const source = program.getSourceFile(path), checker = program.getTypeChecker();
    const calls = [];
    const visit = node => {
      if (ts.isCallExpression(node)) calls.push(node);
      ts.forEachChild(node, visit);
    };
    visit(source);
    assert.equal(calls.length, (test.calls || []).length, `${test.name}: every call needs an independent label`);
    for (const label of test.calls || []) {
      const matches = calls.filter(call => call.getText(source) === label.expression);
      assert.equal(matches.length, 1, test.name);
      const call = matches[0], selector = ts.isPropertyAccessExpression(call.expression) ? call.expression.name : call.expression;
      const declarations = checker.getSymbolAtLocation(selector)?.declarations || [];
      assert.equal(declarations.length, 1, test.name);
      assert(declarations[0].getSourceFile() === source, test.name);
      const expected = text.indexOf(label.declaration);
      assert(expected >= 0 && text.indexOf(label.declaration, expected + 1) < 0, test.name);
      assert.equal(declarations[0].getStart(source), expected, test.name);
      assert(checker.getResolvedSignature(call)?.declaration === declarations[0], test.name);
      exactCallTargets++;
    }
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, scopeVariants: 2,
  positives: cases.filter(t => t.admitted).length,
  diagnosticBackedNegatives: cases.filter(t => t.diagnosticCodes.length).length,
  legalAbstentions: cases.filter(t => t.unsupported).length, exactCallTargets }));
