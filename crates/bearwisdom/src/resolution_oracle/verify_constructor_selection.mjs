import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const cases = JSON.parse(readFileSync(new URL('constructor_selection_fixtures.json', import.meta.url), 'utf8'));
let exactTypes = 0, exactOrigins = 0, implicitOrigins = 0, exactCalls = 0;
for (const test of cases) {
  const absolute = file => resolve('bearwisdom-constructor-selection-virtual', file).replaceAll('\\', '/');
  const sources = new Map(Object.entries(test.sources).map(([file, text]) => [absolute(file), text]));
  const options = { strict: true, noEmit: true, target: ts.ScriptTarget.ESNext, lib: ['lib.es5.d.ts'] };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (file, ...args) => sources.has(file.replaceAll('\\', '/'))
    ? ts.createSourceFile(file, sources.get(file.replaceAll('\\', '/')), options.target, true) : read(file, ...args);
  const program = ts.createProgram(test.roots.map(absolute), options, host);
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a,b) => a-b), test.diagnosticCodes, test.name);
  const source = program.getSourceFile(absolute('main.ts')), checker = program.getTypeChecker();
  let actual, expected;
  const visit = node => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'actual') actual = node;
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'expected') expected = node;
    ts.forEachChild(node, visit);
  }; visit(source);
  assert(actual && ts.isNewExpression(actual.initializer), test.name);
  if (test.diagnosticCodes.length) continue;
  const origin = checker.getResolvedSignature(actual.initializer)?.declaration;
  if (test.selected === null) { assert.equal(origin, undefined, test.name); implicitOrigins++; }
  else {
    assert(origin, test.name);
    assert.equal(origin.getSourceFile().fileName.replaceAll('\\', '/'), absolute(test.selectedFile), test.name);
    assert.equal(origin.getStart(), test.sources[test.selectedFile].indexOf(test.selected), test.name); exactOrigins++;
  }
  const got = checker.getTypeAtLocation(actual.name), wanted = checker.getTypeAtLocation(expected.name);
  assert(got === wanted, `${test.name}: ${checker.typeToString(got)} != ${checker.typeToString(wanted)}`); exactTypes++;
  const calls = node => {
    if (ts.isCallExpression(node)) {
      const declaration = checker.getResolvedSignature(node)?.declaration; assert(declaration?.name, test.name);
      const provider = declaration.getSourceFile();
      assert(sources.has(provider.fileName.replaceAll('\\', '/')), test.name);
      assert.equal(declaration.getStart(), provider.text.indexOf(declaration.name.getText() + '('), test.name); exactCalls++;
    }
    ts.forEachChild(node, calls);
  }; calls(source);
  const order = program.getSourceFiles().filter(f => sources.has(f.fileName.replaceAll('\\', '/'))).map(f => f.fileName.replaceAll('\\', '/'));
  assert.deepEqual(order, test.roots.map(absolute), test.name);
}
console.log(JSON.stringify({compiler:ts.version,cases:cases.length,exactTypes,exactOrigins,implicitOrigins,exactCalls}));
