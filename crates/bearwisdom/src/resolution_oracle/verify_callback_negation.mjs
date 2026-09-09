// Pinned compiler callback types and predicate parameter/declaration evidence.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const cases = JSON.parse(readFileSync(new URL('callback_negation_fixtures.json', import.meta.url), 'utf8'));
let exactCallbacks = 0, exactPredicates = 0, selectedSignatures = 0, exactResults = 0;
for (const test of cases) {
  for (const prefix of ['', 'export {}; ']) {
    const path = resolve('bearwisdom-negation-virtual.ts').replaceAll('\\', '/');
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
    let callback, expected, result, expectedResult;
    const visit = node => {
      if (ts.isArrowFunction(node)) { assert(!callback, test.name); callback = node; }
      if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'expected') expected = node;
      if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'result') result = node.initializer;
      if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'expectedResult') expectedResult = node;
      ts.forEachChild(node, visit);
    };
    visit(source);
    const signature = checker.getSignatureFromDeclaration(callback);
    assert.equal(signature.declaration, callback, test.name);
    assert.equal(checker.typeToString(checker.getReturnTypeOfSignature(signature)), 'boolean', test.name);
    const predicate = checker.getTypePredicateOfSignature(signature);
    assert.equal(!!predicate, test.predicate, test.name);
    if (predicate) {
      assert.equal(predicate.kind, ts.TypePredicateKind.Identifier, test.name);
      assert.equal(predicate.parameterIndex, 0, test.name);
      const wanted = checker.getTypeAtLocation(expected.name);
      assert(predicate.type === wanted, `${test.name}: ${checker.typeToString(predicate.type)} != ${checker.typeToString(wanted)}`);
      exactPredicates++;
    }
    const selected = checker.getResolvedSignature(result)?.declaration;
    assert(selected && selected.getSourceFile() === source, test.name);
    assert.equal(selected.getStart(source), text.indexOf(test.selected), test.name);
    const actual = checker.getTypeAtLocation(result), wanted = checker.getTypeAtLocation(expectedResult.name);
    assert(actual === wanted, `${test.name}: ${checker.typeToString(actual)} != ${checker.typeToString(wanted)}`);
    exactCallbacks++; selectedSignatures++; exactResults++;
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, scopeVariants: 2, exactCallbacks, exactPredicates, selectedSignatures, exactResults,
  diagnosticBackedNegatives: cases.filter(t => t.diagnosticCodes.length).length }));
