// Compiler-owned declaration positions independently validate callable source recipes.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2] || 'typescript');
assert.equal(ts.version, '5.9.3');
const cases = JSON.parse(readFileSync(new URL('./callable_identity_fixtures.json', import.meta.url), 'utf8'));
let signatures = 0, parameters = 0, predicates = 0;
for (const test of cases) {
  const path = resolve('bearwisdom-callable-virtual.ts').replaceAll('\\', '/');
  const options = { strict: true, noEmit: true, target: ts.ScriptTarget.ESNext };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (file, ...args) => file.replaceAll('\\', '/') === path
    ? ts.createSourceFile(path, test.source, options.target, true) : read(file, ...args);
  const program = ts.createProgram([path], options, host);
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code).sort((a,b) => a-b), test.diagnosticCodes, test.name);
  if (test.diagnosticCodes.length) continue;
  const source = program.getSourceFile(path), checker = program.getTypeChecker(), nodes = [];
  const visit = node => { if (ts.isFunctionTypeNode(node)) nodes.push(node); ts.forEachChild(node, visit); };
  visit(source);
  assert.equal(nodes.length, test.signatures.length, test.name);
  for (let i = 0; i < nodes.length; i++) {
    const node = nodes[i], label = test.signatures[i], signature = checker.getSignatureFromDeclaration(node);
    assert(signature && signature.getDeclaration() === node, test.name);
    assert.equal(node.getStart(source), test.source.indexOf(label.start), test.name);
    assert.equal(signature.typeParameters?.length ?? 0, label.generics, test.name);
    const actual = [...(signature.thisParameter ? [signature.thisParameter] : []), ...signature.getParameters()];
    assert.equal(actual.length, label.parameters.length, test.name);
    actual.forEach((symbol, index) => {
      const p = label.parameters[index], declaration = symbol.valueDeclaration;
      assert.equal(declaration.getStart(source), test.source.indexOf(p.start), test.name);
      assert.equal(!!declaration.questionToken, p.optional, test.name);
      assert.equal(!!declaration.dotDotDotToken, p.rest, test.name);
      assert.equal(symbol === signature.thisParameter, p.receiver, test.name);
      parameters++;
    });
    const predicate = checker.getTypePredicateOfSignature(signature);
    assert.equal(!!predicate, !!label.predicate, test.name);
    if (predicate) {
      assert.equal(predicate.kind === ts.TypePredicateKind.AssertsIdentifier, label.predicate.asserts, test.name);
      assert.equal(!!predicate.type, label.predicate.typed, test.name);
      assert.equal(signature.getParameters()[predicate.parameterIndex].valueDeclaration.getStart(source), test.source.indexOf(label.predicate.target), test.name);
      predicates++;
    }
    signatures++;
  }
}
console.log(JSON.stringify({compiler:ts.version,cases:cases.length,signatures,parameters,predicates,diagnosticNegatives:cases.filter(c=>c.diagnosticCodes.length).length}));
