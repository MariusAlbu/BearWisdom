// Read-only compiler syntax/binder evidence; not an inheritance or 99% gate.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";
const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3");
const cases = JSON.parse(readFileSync(new URL("./structural_type_fixtures.json", import.meta.url), "utf8"));
let genericTargets = 0, mappedOwners = 0, diagnostics = 0;
for (const test of cases) {
  const source = `export type Shape<T, K extends keyof T> = ${test.syntax};`;
  const fileName = resolve("bearwisdom-structural-oracle.ts").replaceAll("\\", "/");
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
  const options = { strict: true, target: ts.ScriptTarget.ESNext, noEmit: true };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (path, ...args) => resolve(path).replaceAll("\\", "/") === fileName ? file : read(path, ...args);
  const program = ts.createProgram([fileName], options, host), checker = program.getTypeChecker();
  const codes = ts.getPreEmitDiagnostics(program).map(d => d.code);
  assert.deepEqual(codes, test.diagnosticCodes || [], test.name); diagnostics += codes.length;
  const owner = file.statements[0]; assert(ts.isTypeAliasDeclaration(owner));
  const parameters = new Map(owner.typeParameters.map((p, index) => [p, ["param", "outer", index]]));
  let mapped = 0;
  const modifier = token => !token ? "preserve" : token.kind === ts.SyntaxKind.MinusToken ? "remove" : "add";
  function shape(node) {
    if (ts.isParenthesizedTypeNode(node)) return shape(node.type);
    if (ts.isMappedTypeNode(node)) {
      const name = `mapped${mapped++}`; mappedOwners++;
      parameters.set(node.typeParameter, ["param", name, 0]);
      return ["mapped", modifier(node.readonlyToken), modifier(node.questionToken),
        shape(node.typeParameter.constraint), node.nameType ? shape(node.nameType) : null, shape(node.type)];
    }
    if (ts.isTypeLiteralNode(node)) return ["object", ...node.members.map(member => {
      assert(ts.isPropertySignature(member));
      assert(ts.isIdentifier(member.name));
      return [member.name.text, !!member.questionToken,
        !!member.modifiers?.some(m => m.kind === ts.SyntaxKind.ReadonlyKeyword), shape(member.type)];
    })];
    if (ts.isTypeReferenceNode(node)) {
      const declarations = checker.getSymbolAtLocation(node.typeName)?.declarations || [];
      assert.equal(declarations.length, 1); assert(parameters.has(declarations[0]), "exact source generic owner required");
      genericTargets++; return parameters.get(declarations[0]);
    }
    if (ts.isIndexedAccessTypeNode(node)) return ["index", shape(node.objectType), shape(node.indexType)];
    if (ts.isTypeOperatorNode(node)) { assert.equal(node.operator, ts.SyntaxKind.KeyOfKeyword); return ["keyof", shape(node.type)]; }
    if (ts.isIntersectionTypeNode(node)) return ["intersection", ...node.types.map(shape)];
    if (ts.isLiteralTypeNode(node) && ts.isStringLiteral(node.literal)) return ["string", node.literal.text];
    assert.fail(ts.SyntaxKind[node.kind]);
  }
  assert.deepEqual(shape(owner.type), test.shape, test.name);
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, mappedOwners, genericTargets, diagnostics }));
