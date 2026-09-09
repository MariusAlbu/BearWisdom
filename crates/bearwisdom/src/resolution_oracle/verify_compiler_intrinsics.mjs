// Independent TypeScript diagnostics and alias types; no engine output is used.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const cases = JSON.parse(readFileSync(new URL('compiler_intrinsic_fixtures.json', import.meta.url), 'utf8'));
let typeLabels = 0;
let exactCallTargets = 0;
for (const test of cases) {
  for (const prefix of ['', 'export {}; ']) {
    const path = resolve('bearwisdom-intrinsic-virtual.ts').replaceAll('\\', '/');
    const text = prefix + test.source;
    const options = { noEmit: true, lib: ['lib.es5.d.ts'], ...test.options };
    const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
    host.getSourceFile = (file, ...args) => file.replaceAll('\\', '/') === path
      ? ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true) : read(file, ...args);
    const program = ts.createProgram([path], options, host);
    assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b),
      [...test.diagnosticCodes].sort((a, b) => a - b), test.name + prefix);
    const source = program.getSourceFile(path), checker = program.getTypeChecker(), aliases = [], calls = [];
    const walk = node => { if (ts.isTypeAliasDeclaration(node)) aliases.push(node); if (ts.isCallExpression(node)) calls.push(node); ts.forEachChild(node, walk); };
    walk(source);
    for (const [declaration, type] of test.types || []) {
      const start = text.indexOf(declaration);
      assert(start >= 0 && text.indexOf(declaration, start + 1) < 0, test.name);
      const matches = aliases.filter(node => node.getStart(source) === start);
      assert.equal(matches.length, 1, test.name);
      assert.equal(checker.typeToString(checker.getDeclaredTypeOfSymbol(checker.getSymbolAtLocation(matches[0].name))), type, test.name);
      typeLabels++;
    }
    assert.equal(calls.length, (test.calls || []).length, test.name);
    for (const label of test.calls || []) {
      const matches = calls.filter(call => call.getText(source) === label.expression);
      assert.equal(matches.length, 1, test.name);
      const call = matches[0], selector = call.expression.name;
      const declarations = checker.getSymbolAtLocation(selector)?.declarations || [];
      assert.equal(declarations.length, 1, test.name);
      assert(declarations[0].getSourceFile() === source, test.name);
      const start = text.indexOf(label.declaration);
      assert(start >= 0 && text.indexOf(label.declaration, start + 1) < 0, test.name);
      assert.equal(declarations[0].getStart(source), start, test.name);
      assert(checker.getResolvedSignature(call)?.declaration === declarations[0], test.name);
      exactCallTargets++;
    }
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, scopeVariants: 2, typeLabels, exactCallTargets,
  diagnosticNegatives: cases.filter(t => t.diagnosticCodes.length).length }));
