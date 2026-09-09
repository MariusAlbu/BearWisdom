import assert from "node:assert/strict";
import test from "node:test";
import { createRequire } from "node:module";
import { captureCalls } from "./typescript_project_targets.mjs";
const ts = createRequire(import.meta.url)(process.argv[2] || "typescript");

function calls(source) {
  const sf = ts.createSourceFile("oracle.ts", source, ts.ScriptTarget.Latest, true);
  const host = ts.createCompilerHost({ noLib:true, strict:true }); host.getSourceFile = path => path === sf.fileName ? sf : undefined;
  const program = ts.createProgram([sf.fileName], { noLib:true, strict:true }, host);
  return captureCalls(ts, program, new Map([[sf.fileName, { id:1, selected:true }]]));
}

test("enumeration retains unknown, indirect and multi-declaration calls", () => {
  const found = calls("function over(x:string):void; function over(x:number):void; function over(x:any) {}\nfunction one(){} one(); missing(); over(1); (()=>{})();");
  assert.equal(found.length,4);
  assert.deepEqual(found.map(c=>c.reason||"labelled"),["labelled","no_compiler_target","multiple_declarations","unsupported_call_form"]);
  assert(found.filter(c=>c.reason).every(c=>c.target===undefined));
});
test("compiler declaration positions retain Unicode bytes and skip export wrappers", () => {
  const source="/*é🦀*/ export async function go() {} go();";
  const [found]=calls(source);
  assert.deepEqual(found.target,{file:1,line:0,col:Buffer.byteLength("/*é🦀*/ export "),kind:"function"});
  assert.equal(found.site.byte_offset,Buffer.byteLength(source.slice(0,source.lastIndexOf("go()"))));
});
test("namesake scopes use different declaration coordinates", () => {
  const found=calls("function one(){ function f(){} f(); } function two(){ function f(){} f(); }");
  assert.equal(found.length,2); assert.notDeepEqual(found[0].target,found[1].target);
});
