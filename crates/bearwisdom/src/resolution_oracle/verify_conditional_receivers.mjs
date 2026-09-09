// Independent compiler declarations, selected signatures and result types.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const cases = JSON.parse(readFileSync(new URL('conditional_receiver_fixtures.json', import.meta.url), 'utf8'));
let exactDeclarations = 0, exactSignatures = 0, resultTypes = 0;
for (const test of cases) {
  for (const prefix of ['', 'export {}; ']) {
    const path = resolve('bearwisdom-conditional-receiver-virtual.ts').replaceAll('\\', '/');
    const text = prefix + test.source;
    const options = { noEmit: true, strict: true, lib: ['lib.es5.d.ts'] };
    const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
    host.getSourceFile = (file, ...args) => file.replaceAll('\\', '/') === path
      ? ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true) : read(file, ...args);
    const program = ts.createProgram([path], options, host);
    assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b),
      [...test.diagnosticCodes].sort((a, b) => a - b), test.name + prefix);
    const source = program.getSourceFile(path), checker = program.getTypeChecker(), calls = [];
    const walk = node => { if (ts.isCallExpression(node)) calls.push(node); ts.forEachChild(node, walk); };
    walk(source);
    assert.equal(calls.length, test.calls.length, test.name);
    for (const label of test.calls) {
      const matches = calls.filter(call => call.getText(source) === label.expression);
      assert.equal(matches.length, 1, test.name);
      if (!label.declaration) continue;
      const call = matches[0], declarations = checker.getSymbolAtLocation(call.expression.name)?.declarations || [];
      assert.equal(declarations.length, 1, test.name);
      const start = text.indexOf(label.declaration);
      assert(start >= 0 && text.indexOf(label.declaration, start + 1) < 0, test.name);
      assert(declarations[0].getSourceFile() === source, test.name);
      assert.equal(declarations[0].getStart(source), start, test.name); exactDeclarations++;
      assert(checker.getResolvedSignature(call)?.declaration === declarations[0], test.name); exactSignatures++;
      assert.equal(checker.typeToString(checker.getTypeAtLocation(call)), label.result, test.name); resultTypes++;
    }
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, scopeVariants: 2,
  exactDeclarations, exactSignatures, resultTypes, supportedCases: cases.filter(c => c.supported).length,
  diagnosticNegatives: cases.filter(c => c.diagnosticCodes.length).length }));
