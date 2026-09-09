// Independent type-value evidence: compiler flags and values, never typeToString.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import assert from "node:assert/strict";

const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");
assert.equal(ts.version, "5.9.3");
const cases = JSON.parse(readFileSync(new URL("./atomic_type_fixtures.json", import.meta.url), "utf8"));
const source = cases.map((test, i) => "type Case" + i + " = " + test.syntax + ";").join("\n");
const fileName = resolve("bearwisdom-atomic-oracle.ts").replaceAll("\\", "/");
const options = { strict: true, target: ts.ScriptTarget.ESNext, noEmit: true };
const file = ts.createSourceFile(fileName, source, options.target, true);
const host = ts.createCompilerHost(options), original = host.getSourceFile.bind(host);
host.getSourceFile = (path, ...args) => resolve(path).replaceAll("\\", "/") === fileName ? file : original(path, ...args);
const program = ts.createProgram([fileName], options, host), checker = program.getTypeChecker();
assert.deepEqual(ts.getPreEmitDiagnostics(program).map(d => ({code:d.code, message:d.messageText})), []);
const intrinsicFlags = new Map([
  [ts.TypeFlags.Any, "Any"], [ts.TypeFlags.Unknown, "Unknown"], [ts.TypeFlags.Never, "Never"],
  [ts.TypeFlags.Void, "Void"], [ts.TypeFlags.Undefined, "Undefined"], [ts.TypeFlags.Null, "Null"],
  [ts.TypeFlags.NonPrimitive, "Object"], [ts.TypeFlags.String, "String"], [ts.TypeFlags.Number, "Number"],
  [ts.TypeFlags.Boolean, "Boolean"], [ts.TypeFlags.ESSymbol, "Symbol"], [ts.TypeFlags.BigInt, "BigInt"],
]);
assert.equal(file.statements.length, cases.length);
let checked = 0;
for (const [index, declaration] of file.statements.entries()) {
  assert(ts.isTypeAliasDeclaration(declaration));
  const type = checker.getTypeFromTypeNode(declaration.type), flags = type.flags;
  let actual;
  if (flags & ts.TypeFlags.StringLiteral) {
    actual = {string: Array.from({length:type.value.length}, (_, i) => type.value.charCodeAt(i))};
  } else if (flags & ts.TypeFlags.NumberLiteral) {
    const bytes = Buffer.alloc(8); bytes.writeDoubleBE(type.value === 0 ? 0 : type.value);
    actual = {number:bytes.toString("hex")};
  } else if (flags & ts.TypeFlags.BigIntLiteral) {
    let magnitude = BigInt(type.value.base10Value), words = [];
    while (magnitude) { words.push(Number(magnitude & 0xffffffffn)); magnitude >>= 32n; }
    actual = {bigint:{negative:type.value.negative && words.length > 0, words}};
  } else if (flags & ts.TypeFlags.BooleanLiteral) {
    assert(["true", "false"].includes(type.intrinsicName));
    actual = {boolean:type.intrinsicName === "true"};
  } else {
    const identities = [...intrinsicFlags].filter(([flag]) => flags & flag);
    assert.equal(identities.length, 1, "Unexpected compiler flags: " + flags);
    actual = {intrinsic:identities[0][1]};
  }
  assert.deepEqual(actual, cases[index].expected, cases[index].syntax); checked++;
}
console.log(JSON.stringify({compiler:ts.version, atomicTypesVerified:checked, compilerDiagnostics:0}));
