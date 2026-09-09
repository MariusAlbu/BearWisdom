// Compiler dispatch evidence is independent of engine call-symbol navigation.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const cases = [...JSON.parse(readFileSync(new URL('./overload_call_fixtures.json', import.meta.url), 'utf8')),
  ...JSON.parse(readFileSync(new URL('./overload_order_fixtures.json', import.meta.url), 'utf8')).map(test => ({...test, supported:true, diagnosticCodes:[]}))];
let selectedSignatures = 0, exactTypes = 0;
for (const test of cases) {
  const address = name => resolve('bearwisdom-overload-virtual', name).replaceAll('\\', '/');
  const path = address('main.ts');
  const texts = new Map(Object.entries(test.sources || {'main.ts':test.source}).map(([name, text]) => [address(name), text]));
  const options = { strict: true, noEmit: true, target: ts.ScriptTarget.ESNext };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (file, ...args) => texts.has(file.replaceAll('\\', '/'))
    ? ts.createSourceFile(file, texts.get(file.replaceAll('\\', '/')), options.target, true) : read(file, ...args);
  const roots = (test.roots || ['main.ts']).map(address);
  const program = ts.createProgram(roots, options, host);
  assert.deepEqual(program.getSourceFiles().filter(sf => texts.has(sf.fileName.replaceAll('\\','/'))).map(sf => sf.fileName.replaceAll('\\','/')), roots);
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a, b) => a - b),
    [...test.diagnosticCodes].sort((a, b) => a - b), test.name);
  if (test.diagnosticCodes.length) continue;
  const source = program.getSourceFile(path), checker = program.getTypeChecker();
  let call, expected;
  const visit = node => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'result') call = node.initializer;
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'expected') expected = node;
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert(ts.isCallExpression(call) && expected, test.name);
  const signature = checker.getResolvedSignature(call), declaration = signature?.getDeclaration();
  const selectedSource = program.getSourceFile(address(test.selectedFile || 'main.ts'));
  assert(declaration && declaration.getSourceFile() === selectedSource, test.name);
  assert.equal(declaration.getStart(selectedSource), selectedSource.text.indexOf(test.selected), test.name);
  const actual = checker.getTypeAtLocation(call), wanted = checker.getTypeAtLocation(expected.name);
  assert.equal(actual, wanted, `${test.name}: ${checker.typeToString(actual)} !== ${checker.typeToString(wanted)}`);
  selectedSignatures++; exactTypes++;
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, selectedSignatures, exactTypes,
  diagnosticBackedNegatives: cases.filter(t => t.diagnosticCodes.length).length,
  legalAbstentions: cases.filter(t => !t.supported && !t.diagnosticCodes.length).length }));
