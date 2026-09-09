// Independent compiler results, declaration origins and diagnostic labels.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const cases = JSON.parse(readFileSync(new URL('structural_initializer_fixtures.json', import.meta.url), 'utf8'));
let exactTypes = 0, constructorSignatures = 0, exactCallTargets = 0;
for (const test of cases) {
  for (const prefix of ['', 'export {}; ']) {
    const path = resolve('bearwisdom-structural-virtual.ts').replaceAll('\\', '/');
    const text = prefix + test.source;
    const options = { strict: true, noEmit: true, target: ts.ScriptTarget.ESNext, lib: ['lib.es5.d.ts'] };
    const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
    host.getSourceFile = (file, ...args) => file.replaceAll('\\', '/') === path
      ? ts.createSourceFile(path, text, options.target, true) : read(file, ...args);
    const program = ts.createProgram([path], options, host);
    const diagnostics = ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b);
    assert.deepEqual(diagnostics, [...test.diagnosticCodes].sort((a, b) => a - b), `${test.name} (${prefix})`);
    if (!test.supported) { assert(diagnostics.length, test.name); continue; }
    assert.equal(diagnostics.length, 0, test.name);
    const source = program.getSourceFile(path), checker = program.getTypeChecker();
    let field, expected; const calls = [];
    const visit = node => {
      if (ts.isPropertyDeclaration(node) && node.name.getText(source) === 'value') field = node;
      if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'expected') expected = node;
      if (ts.isCallExpression(node)) calls.push(node);
      ts.forEachChild(node, visit);
    };
    visit(source);
    assert(field && expected, test.name);
    const actual = checker.getTypeAtLocation(field.name), wanted = checker.getTypeAtLocation(expected.name);
    assert(actual === wanted, `${test.name}: ${checker.typeToString(actual)} != ${checker.typeToString(wanted)}`);
    const signature = checker.getResolvedSignature(field.initializer);
    assert(signature?.declaration && ts.isConstructSignatureDeclaration(signature.declaration), test.name);
    assert.equal(signature.declaration.getStart(source), text.indexOf('new<U>'), test.name);
    assert.equal(signature.declaration.getSourceFile(), source, test.name);
    exactTypes++; constructorSignatures++;
    assert.equal(calls.length, (test.calls || []).length, test.name);
    for (const label of test.calls || []) {
      const matches = calls.filter(call => call.getText(source) === label.expression);
      assert.equal(matches.length, 1, test.name);
      const call = matches[0], selector = call.expression.name;
      const symbol = checker.getSymbolAtLocation(selector), selected = checker.getResolvedSignature(call);
      const offset = text.indexOf(label.declaration);
      assert(symbol?.declarations?.some(d => d.getSourceFile() === source && d.getStart(source) === offset), test.name);
      assert.equal(selected?.declaration?.getSourceFile(), source, test.name);
      assert.equal(selected?.declaration?.getStart(source), offset, test.name);
      exactCallTargets++;
    }
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, scopeVariants: 2, exactTypes, constructorSignatures, exactCallTargets,
  diagnosticBackedNegatives: cases.filter(t => t.diagnosticCodes.length).length }));
