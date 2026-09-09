// Independent AST evidence, not a declaration-binding or merge-legality oracle.
// Usage: node verify_member_surfaces.mjs <typescript.js> <captured-report.json>
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { createHash } from 'node:crypto';
import { isDeepStrictEqual } from 'node:util';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2]);
assert.equal(ts.version, '5.9.3', 'Review adapter before changing compiler pin');
const report = JSON.parse(readFileSync(process.argv[3], 'utf8'));
assert(report.files.length > 0, 'Empty capture report');
let checked = 0;
const failures = [];
const kinds = new Map([
  [ts.SyntaxKind.PropertySignature, 'Property'], [ts.SyntaxKind.PropertyDeclaration, 'Property'],
  [ts.SyntaxKind.MethodSignature, 'Method'], [ts.SyntaxKind.MethodDeclaration, 'Method'],
  [ts.SyntaxKind.GetAccessor, 'Getter'], [ts.SyntaxKind.SetAccessor, 'Setter'],
  [ts.SyntaxKind.CallSignature, 'Call'], [ts.SyntaxKind.ConstructSignature, 'Construct'],
  [ts.SyntaxKind.Constructor, 'Construct'], [ts.SyntaxKind.IndexSignature, 'Index'],
]);
const modifiers = new Map([
  [ts.SyntaxKind.ReadonlyKeyword, 'Readonly'], [ts.SyntaxKind.StaticKeyword, 'Static'],
  [ts.SyntaxKind.AbstractKeyword, 'Abstract'], [ts.SyntaxKind.PublicKeyword, 'Public'],
  [ts.SyntaxKind.ProtectedKeyword, 'Protected'], [ts.SyntaxKind.PrivateKeyword, 'Private'],
  [ts.SyntaxKind.OverrideKeyword, 'Override'], [ts.SyntaxKind.DeclareKeyword, 'Declare'],
]);
for (const file of report.files) {
  const source = file.source ?? readFileSync(file.path, 'utf8');
  assert.equal(createHash('sha256').update(source).digest('hex'), file.sha256, 'Changed compiler input');
  const tree = ts.createSourceFile(file.name ?? file.path, source, ts.ScriptTarget.Latest, true);
  assert.equal(tree.parseDiagnostics.length, 0, file.name ?? file.path);
  // Build UTF-16 -> UTF-8 coordinates once, not a prefix encoding per node.
  const offsets = new Uint32Array(source.length + 1);
  let bytes = 0;
  for (let i = 0; i < source.length; i++) {
    offsets[i] = bytes;
    const code = source.charCodeAt(i);
    if (code >= 0xd800 && code <= 0xdbff && source.charCodeAt(i + 1) >= 0xdc00 && source.charCodeAt(i + 1) <= 0xdfff) {
      offsets[++i] = bytes; bytes += 4;
    } else bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : 3;
  }
  offsets[source.length] = bytes;
  const span = node => node ? ({ start: offsets[node.getStart(tree)], end: offsets[node.end] }) : null;
  const expected = [];
  // Global source declarations only; no nested namespace/local lookalikes.
  for (const owner of tree.statements) {
    if (!ts.isInterfaceDeclaration(owner) && !ts.isClassDeclaration(owner)) continue;
    for (const member of owner.members) {
      expected.push({ start: span(member).start, kind: kinds.get(member.kind) ?? 'Unknown',
        modifiers: [...Array.from(member.modifiers ?? [], m => modifiers.get(m.kind) ?? 'Unknown'),
          ...(member.questionToken ? ['Optional'] : [])],
        key_span: ts.isIndexSignatureDeclaration(member) ? null : span(member.name),
        result: span(member.type),
        type_parameters: Array.from(member.typeParameters ?? [], span),
        parameters: Array.from(member.parameters ?? [], p => ({ span: span(p), type_span: span(p.type),
          optional: !!p.questionToken, rest: !!p.dotDotDotToken })),
        initializer: span(member.initializer),
        unique_symbol: !!member.type && ts.isTypeOperatorNode(member.type)
          && member.type.operator === ts.SyntaxKind.UniqueKeyword && member.type.type.kind === ts.SyntaxKind.SymbolKeyword,
      });
    }
  }
  const actual = file.members.map(m => ({ start: m.span.start, kind: m.kind,
    modifiers: m.modifiers,
    key_span: m.key_span, result: m.signature.result, type_parameters: m.signature.type_parameters,
    parameters: m.signature.parameters, initializer: m.signature.initializer, unique_symbol: m.signature.unique_symbol }));
  if (!isDeepStrictEqual(actual, expected)) {
    const differences = Array.from({ length: Math.max(actual.length, expected.length) }, (_, i) => i)
      .filter(i => !isDeepStrictEqual(actual[i], expected[i]));
    failures.push({ file: file.name ?? file.path, captured: actual.length, compiler: expected.length,
      differences: differences.length, first: differences.slice(0, 2).map(i => ({ actual: actual[i], expected: expected[i] })) });
  }
  checked += expected.length;
}
if (failures.length) console.error(JSON.stringify(failures));
assert.equal(failures.length, 0, 'Compiler member-surface disagreements');
console.log(JSON.stringify({ compiler: 'TypeScript', version: ts.version, files: report.files.length,
  verifiedMemberSurfaces: checked, evidence: 'source AST only; no merge compatibility or binding claim' }));
