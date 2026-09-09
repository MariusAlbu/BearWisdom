// Read-only AST, generic-target and conditional-distribution controls. These
// check source recipes, NOT operator evaluation or merge compatibility.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";
const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3");
const cases = JSON.parse(readFileSync(new URL("./operator_type_fixtures.json", import.meta.url), "utf8"));
let genericTargets = 0, operators = 0;
for (const test of cases) {
  const source = `export interface Ops<T, K extends keyof T> { value: ${test.syntax}; }`;
  const fileName = resolve("bearwisdom-operator-oracle.ts").replaceAll("\\", "/");
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.Latest, true);
  const options = { strict: true, target: ts.ScriptTarget.ESNext, noEmit: true };
  const host = ts.createCompilerHost(options), read = host.getSourceFile.bind(host);
  host.getSourceFile = (path, ...args) => resolve(path).replaceAll("\\", "/") === fileName ? file : read(path, ...args);
  const program = ts.createProgram([fileName], options, host), checker = program.getTypeChecker();
  assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => d.code), [], test.syntax);
  const owner = file.statements[0]; assert(ts.isInterfaceDeclaration(owner));
  function shape(node) {
    if (ts.isParenthesizedTypeNode(node)) return shape(node.type);
    if (ts.isTypeOperatorNode(node)) {
      operators++;
      const kind = node.operator === ts.SyntaxKind.KeyOfKeyword ? "keyof" : node.operator === ts.SyntaxKind.ReadonlyKeyword ? "readonly" : null;
      assert(kind); return [kind, shape(node.type)];
    }
    if (ts.isIndexedAccessTypeNode(node)) { operators++; return ["index", shape(node.objectType), shape(node.indexType)]; }
    if (ts.isConditionalTypeNode(node)) {
      operators++;
      const type = checker.getTypeFromTypeNode(node);
      assert(type.flags & ts.TypeFlags.Conditional, "fixture should retain a deferred conditional");
      assert.equal(typeof type.root.isDistributive, "boolean");
      return ["conditional", type.root.isDistributive, shape(node.checkType), shape(node.extendsType), shape(node.trueType), shape(node.falseType)];
    }
    if (ts.isTypeReferenceNode(node)) {
      const declarations = checker.getSymbolAtLocation(node.typeName)?.declarations || [];
      assert.equal(declarations.length, 1);
      const index = owner.typeParameters.indexOf(declarations[0]); assert(index >= 0, "must bind this exact source generic owner");
      genericTargets++; return ["param", index];
    }
    if (ts.isTupleTypeNode(node)) return ["tuple", ...node.elements.map(shape)];
    if (ts.isArrayTypeNode(node)) return ["array", shape(node.elementType)];
    if (ts.isLiteralTypeNode(node) && ts.isNumericLiteral(node.literal)) return ["number", Number(node.literal.text)];
    const atoms = new Map([[ts.SyntaxKind.NeverKeyword, "never"], [ts.SyntaxKind.StringKeyword, "string"], [ts.SyntaxKind.NumberKeyword, "number"]]);
    assert(atoms.has(node.kind), ts.SyntaxKind[node.kind]); return ["intrinsic", atoms.get(node.kind)];
  }
  assert.deepEqual(shape(owner.members[0].type), test.shape, test.syntax);
}
console.log(JSON.stringify({ compiler: ts.version, cases: cases.length, operatorNodesVerified: operators, genericTargetsVerified: genericTargets, diagnostics: 0 }));
