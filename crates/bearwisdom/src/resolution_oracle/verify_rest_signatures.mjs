// Independent compiler relations for ordinary, construct and method signatures.
import { readFileSync, writeFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const fixture = new URL('rest_signature_fixtures.json', import.meta.url);
const cases = JSON.parse(readFileSync(fixture, 'utf8'));
const capture = process.argv.includes('--capture');
let assignments = 0, negatives = 0;
for (const test of cases) for (const kind of ['callable', 'constructor', 'method']) for (const prefix of ['', 'export {}; ']) {
  const signature = (side, name) => {
    const parameters = test[side + 'Parameters'], generics = test[side + 'Generics'] || '';
    const result = test[side + 'Result'] || 'Item';
    return kind === 'callable' ? `type ${name} = ${generics}(${parameters}) => ${result};`
      : `interface ${name} { ${kind === 'constructor' ? 'new' : 'invoke'}${generics}(${parameters}): ${result} }`;
  };
  const text = prefix + `interface Array<T> { length: number } interface ReadonlyArray<T> { readonly length: number } interface Item { touch(): number } interface Rich extends Item { rich(): string } ${test.preamble || ''} ${signature('source', 'From')} ${signature('target', 'To')} declare const from: From; const actual: To = from;`;
  const path = resolve('bearwisdom-rest-signatures-virtual.ts').replaceAll('\\', '/');
  const options = { noEmit: true, strict: true, strictFunctionTypes: test.strict ?? true, strictNullChecks: test.strictNulls ?? true, lib: ['lib.es5.d.ts'] };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (file, ...args) => file.replaceAll('\\', '/') === path
    ? ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true) : read(file, ...args);
  const program = ts.createProgram([path], options, host), source = program.getSourceFile(path), checker = program.getTypeChecker();
  const diagnostics = ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a,b) => a-b);
  const declarations = new Map(source.statements.filter(n => ts.isTypeAliasDeclaration(n) || ts.isInterfaceDeclaration(n)).map(n => [n.name.text, n]));
  const type = name => checker.getTypeAtLocation(declarations.get(name).name);
  const label = { diagnostics, assignable: checker.isTypeAssignableTo(type('From'), type('To')) };
  if (capture && !prefix) (test.labels ||= {})[kind] = label;
  assert.deepEqual(label, test.labels?.[kind], test.name + ':' + kind + ':' + prefix);
  if (test.supported !== false) assert(diagnostics.every(code => code === 2322), test.name + ': invalid source');
  assignments++; if (!label.assignable) negatives++;
}
if (capture) writeFileSync(fixture, JSON.stringify(cases, null, 2) + '\n');
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, signatureKinds: 3, scopeVariants: 2, assignments, negatives }));
