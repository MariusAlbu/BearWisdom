// Independent compiler types, signature presence and diagnostics; no engine inputs.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const files = ['initializer_fixtures.json', 'array_initializer_fixtures.json'];
const cases = files.flatMap(file => JSON.parse(readFileSync(new URL(file, import.meta.url), 'utf8')));
let exactTypes = 0, constructorSignatures = 0;
for (const test of cases) {
  console.error(`compiler initializer: ${test.name}`);
  const path = resolve('bearwisdom-initializer-virtual.ts').replaceAll('\\', '/');
  const options = { strict: true, noEmit: true, target: ts.ScriptTarget.ESNext };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (file, ...args) => file.replaceAll('\\', '/') === path
    ? ts.createSourceFile(path, `export {}; ${test.source}`, options.target, true) : read(file, ...args);
  const program = ts.createProgram([path], options, host);
  const diagnostics = ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b);
  assert.deepEqual(diagnostics, [...test.diagnosticCodes].sort((a, b) => a - b), test.name);
  if (!test.supported && !test.source.includes('declare const expected:')) continue;
  const source = program.getSourceFile(path), checker = program.getTypeChecker();
  let field, expected;
  const visit = node => {
    if (ts.isPropertyDeclaration(node) && node.name.getText(source) === 'value') field = node;
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'expected') expected = node;
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert(field && expected, test.name);
  const actual = checker.getTypeAtLocation(field.name), wanted = checker.getTypeAtLocation(expected.name);
  // Compiler Type objects contain the whole cyclic checker graph. Asking the
  // assertion formatter to diff them can allocate tens of GB on a label error.
  assert(actual === wanted, `${test.name}: ${checker.typeToString(actual)} !== ${checker.typeToString(wanted)}`);
  assert(ts.isNewExpression(field.initializer), test.name);
  const signature = checker.getResolvedSignature(field.initializer);
  assert(signature, test.name);
  exactTypes++; constructorSignatures++;
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, exactTypes, constructorSignatures,
  diagnosticBackedNegatives: cases.filter(t => t.diagnosticCodes.length).length,
  legalAbstentions: cases.filter(t => !t.supported && !t.diagnosticCodes.length).length }));
