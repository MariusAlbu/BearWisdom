import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const cases = JSON.parse(readFileSync(new URL('object_initializer_fixtures.json', import.meta.url), 'utf8'));
let signatures = 0, declarations = 0, results = 0, reads = 0;
for (const test of cases) {
  const path = resolve('bearwisdom-object-initializer-virtual.ts').replaceAll('\\', '/');
  const text = test.source, options = { strict: true, noEmit: true, target: ts.ScriptTarget.ESNext, lib: ['lib.es5.d.ts'] };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (file, ...args) => file.replaceAll('\\', '/') === path ? ts.createSourceFile(path, text, options.target, true) : read(file, ...args);
  const program = ts.createProgram([path], options, host);
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b), [...test.diagnosticCodes].sort((a, b) => a - b), test.name);
  const source = program.getSourceFile(path), checker = program.getTypeChecker(), calls = [];
  const visit = node => { if (ts.isCallExpression(node)) calls.push(node); ts.forEachChild(node, visit); }; visit(source);
  for (const label of test.reads || []) {
    let declaration;
    const find = node => { if (ts.isVariableDeclaration(node) && node.name.getText(source) === label.name) declaration = node; ts.forEachChild(node, find); }; find(source);
    assert(declaration, test.name);
    assert.equal(checker.typeToString(checker.getTypeAtLocation(declaration.name)), label.type, `${test.name}: ${label.name}`); reads++;
  }
  for (const label of test.calls) {
    const call = calls.find(c => c.getText(source) === label.expression); assert(call, test.name);
    if (!label.supported) continue;
    const signature = checker.getResolvedSignature(call)?.declaration;
    assert(signature && signature.getSourceFile() === source, test.name);
    assert.equal(signature.getStart(source), text.indexOf(label.signature), `${test.name}: signature`); signatures++;
    const declaration = checker.getSymbolAtLocation(call.expression.name)?.valueDeclaration;
    assert(declaration && declaration.getSourceFile() === source, test.name);
    assert.equal(declaration.getStart(source), text.indexOf(label.declaration), `${test.name}: declaration`); declarations++;
    assert.equal(checker.typeToString(checker.getTypeAtLocation(call)), label.result, `${test.name}: result`); results++;
  }
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, signatures, declarations, results, reads }));
