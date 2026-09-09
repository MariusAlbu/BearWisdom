// Compiler-owned constructor inventories, assignment legality and selected origins.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const fixture = new URL('constructor_heritage_fixtures.json', import.meta.url);
const cases = JSON.parse(readFileSync(fixture, 'utf8'));
const capture = process.argv.includes('--capture');
let inventorySignatures = 0, selectedOrigins = 0, exactResults = 0, assignments = 0, exactCalls = 0;
for (const test of cases) {
  for (const prefix of ['', 'export {}; ']) {
    const path = resolve('bearwisdom-constructor-heritage-virtual.ts').replaceAll('\\', '/');
    const text = prefix + (test.arrayProvider || '') + test.source;
    const options = { noEmit: true, strict: test.strict ?? true, lib: ['lib.es5.d.ts'] };
    const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
    host.getSourceFile = (file, ...args) => file.replaceAll('\\', '/') === path
      ? ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true) : read(file, ...args);
    const program = ts.createProgram([path], options, host);
    const diagnostics = ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a,b) => a-b);
    if (capture && !prefix) test.diagnosticCodes = diagnostics;
    assert.deepEqual(diagnostics, test.diagnosticCodes, test.name + prefix);
    const source = program.getSourceFile(path), checker = program.getTypeChecker(), declarations = new Map();
    const walk = node => {
      if (ts.isInterfaceDeclaration(node) || ts.isVariableDeclaration(node)) declarations.set(node.name.getText(source), node);
      ts.forEachChild(node, walk);
    }; walk(source);
    const declared = name => checker.getTypeAtLocation(declarations.get(name).name);
    const inventory = type => checker.getSignaturesOfType(type, ts.SignatureKind.Construct).map(signature => {
      const declaration = signature.declaration;
      assert(declaration && declaration.getSourceFile() === source, test.name);
      return { start: declaration.getStart(source) - prefix.length,
        parameters: signature.parameters.map(p => checker.typeToString(checker.getTypeOfSymbolAtLocation(p, declaration))),
        result: checker.typeToString(checker.getReturnTypeOfSignature(signature)) };
    });
    const inventories = test.mode === 'heritage' ? { Factory: inventory(declared('Factory')) }
      : { Source: inventory(declared('Source')), Target: inventory(declared('Target')) };
    if (capture && !prefix) test.inventories = inventories;
    assert.deepEqual(inventories, test.inventories, test.name + ': inventories');
    inventorySignatures += Object.values(inventories).reduce((n, rows) => n + rows.length, 0);
    if (test.mode === 'relation') {
      if (!diagnostics.some(code => code !== 2322)) {
        assert.equal(checker.isTypeAssignableTo(declared('Source'), declared('Target')), test.admitted, test.name);
        assignments++;
      }
    } else if (!diagnostics.length) {
      const actual = declarations.get('actual'), expected = declarations.get('expected');
      const signature = checker.getResolvedSignature(actual.initializer);
      assert.equal(signature.declaration.getStart(source), text.indexOf(test.selected), test.name); selectedOrigins++;
      assert(checker.getTypeAtLocation(actual.name) === checker.getTypeAtLocation(expected.name), test.name); exactResults++;
      const calls = [];
      const visit = node => { if (ts.isCallExpression(node)) calls.push(node); ts.forEachChild(node, visit); }; visit(source);
      assert.equal(calls.length, 1, test.name);
      const call = calls[0]; assert.equal(call.getText(source), test.call.expression);
      assert.equal(checker.getResolvedSignature(call).declaration.getStart(source), text.indexOf(test.call.declaration), test.name);
      assert.equal(checker.typeToString(checker.getTypeAtLocation(call)), test.call.result, test.name); exactCalls++;
    }
  }
}
if (capture) writeFileSync(fixture, JSON.stringify(cases, null, 2) + '\n');
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, scopeVariants: 2,
  inventorySignatures, selectedOrigins, exactResults, assignments, exactCalls,
  diagnosticNegatives: cases.filter(c => c.diagnosticCodes.length).length }));
