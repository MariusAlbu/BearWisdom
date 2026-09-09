// Independent compiler export-set and source-binding evidence; no 99% claim.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

assert.equal(process.argv.length, 3, 'Usage: node verify-module-scopes.mjs <typescript-5.9.3.js>');
const ts = createRequire(import.meta.url)(resolve(process.argv[2]));
assert.equal(ts.version, '5.9.3');
function program(source) {
  const file = ts.createSourceFile('/provider.ts', source, ts.ScriptTarget.Latest, true);
  const host = {
    getSourceFile: name => name === file.fileName ? file : undefined,
    getDefaultLibFileName: () => '', writeFile: () => assert.fail('read-only evidence'),
    getCurrentDirectory: () => '/', getDirectories: () => [],
    fileExists: name => name === file.fileName, readFile: name => name === file.fileName ? source : undefined,
    getCanonicalFileName: name => name, useCaseSensitiveFileNames: () => true, getNewLine: () => '\n',
  };
  const value = ts.createProgram([file.fileName], { noLib: true, strict: true, target: ts.ScriptTarget.ESNext }, host);
  assert.deepEqual(value.getSyntacticDiagnostics(file), [], source);
  assert.deepEqual(value.getSemanticDiagnostics(file), [], source);
  return { file, checker: value.getTypeChecker() };
}
const fixtures = JSON.parse(readFileSync(new URL('./module-scope-fixtures.json', import.meta.url), 'utf8'));
let exportSets = 0;
for (const fixture of fixtures) {
  const { file, checker } = program(fixture.source);
  const actual = [];
  const visit = node => {
    if (ts.isModuleDeclaration(node)) {
      const symbol = checker.getSymbolAtLocation(node.name);
      assert(symbol, fixture.source);
      actual.push(checker.getExportsOfModule(symbol).map(s => s.name).sort());
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  assert.deepEqual(actual, fixture.exports.map(names => [...names].sort()), fixture.source);
  exportSets += actual.length;
}
const source = "declare module 'a' { var item: string; type First = typeof item; } declare module 'b' { var item: number; type Second = typeof item; }";
const { file, checker } = program(source);
const targets = [];
const visit = node => {
  if (ts.isTypeQueryNode(node)) {
    const declarations = checker.getSymbolAtLocation(node.exprName)?.declarations;
    assert.equal(declarations?.length, 1);
    targets.push(declarations[0].name.getStart(file));
  }
  ts.forEachChild(node, visit);
};
visit(file);
assert.deepEqual(targets, [...source.matchAll(/var item/g)].map(match => match.index + 4));
assert.notEqual(targets[0], targets[1]);
console.log(JSON.stringify({ compiler: ts.version, fixtures: fixtures.length, exportSets, scopedVarTargets: targets.length,
  evidence: 'module-export-sets-and-exact-source-bindings; diagnostic-not-representative' }));
