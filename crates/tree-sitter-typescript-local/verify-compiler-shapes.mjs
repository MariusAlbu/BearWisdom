// Independent syntax/ownership evidence only, not a symbol-target or 99% oracle.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';

assert.equal(process.argv.length, 3, 'Usage: node verify-compiler-shapes.mjs <typescript-5.9.3.js>');
const ts = createRequire(import.meta.url)(resolve(process.argv[2]));
assert.equal(ts.version, '5.9.3', 'Compiler evidence is version-pinned');
let checked = 0;
for (const scriptKind of [ts.ScriptKind.TS, ts.ScriptKind.TSX]) {
  const parse = source => {
    const file = ts.createSourceFile('provider.ts', source, ts.ScriptTarget.Latest, true, scriptKind);
    assert.deepEqual(file.parseDiagnostics, [], source);
    checked++;
    return file;
  };
  for (const prefix of ['global', 'declare global']) {
    const file = parse(`declare module 'provider' { ${prefix} { interface Catalog { read(): string; } } }`);
    const module = file.statements[0];
    assert(ts.isModuleDeclaration(module) && ts.isStringLiteral(module.name));
    const augmentation = module.body.statements[0];
    assert(ts.isModuleDeclaration(augmentation));
    assert(augmentation.flags & ts.NodeFlags.GlobalAugmentation);
    assert(ts.isInterfaceDeclaration(augmentation.body.statements[0]));
  }
  const keys = parse('type Keys = keyof readonly string[] | number;').statements[0].type;
  assert(ts.isUnionTypeNode(keys));
  assert.equal(keys.types[0].operator, ts.SyntaxKind.KeyOfKeyword);
  assert.equal(keys.types[0].type.operator, ts.SyntaxKind.ReadonlyKeyword);
  assert(ts.isArrayTypeNode(keys.types[0].type.type));
  const mapped = parse('type Keys = { readonly [K in keyof readonly any[]]?: boolean };').statements[0].type;
  assert(ts.isMappedTypeNode(mapped));
  assert.equal(mapped.typeParameter.constraint.operator, ts.SyntaxKind.KeyOfKeyword);
  const conditional = parse("type Stream<T> = typeof globalThis extends { onmessage: any } ? {} : import('provider').Stream<T>;")
    .statements[0].type;
  assert(ts.isConditionalTypeNode(conditional) && ts.isImportTypeNode(conditional.falseType));
  assert.equal(conditional.falseType.typeArguments.length, 1);
  const array = parse("type Item = import('provider').Stream<string>[number][];").statements[0].type;
  assert(ts.isArrayTypeNode(array) && ts.isIndexedAccessTypeNode(array.elementType));
  assert(ts.isImportTypeNode(array.elementType.objectType));
  parse('const global = () => {}; global(); global: { break global; }');
}
console.log(JSON.stringify({ compiler: ts.version, checked, evidence: 'syntax-and-AST-ownership-only' }));
