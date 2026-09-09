import assert from "node:assert/strict";
import test from "node:test";
import { mkdtempSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createRequire } from "node:module";
import { capture } from "./capture_typescript_project.mjs";
const ts=createRequire(import.meta.url)(process.argv[2]||"typescript");

test("configured source scopes follow compiler module detection, not filename guesses",()=>{
  const root=mkdtempSync(join(tmpdir(),"bw-project-oracle-"));
  try {
    writeFileSync(join(root,"tsconfig.json"),JSON.stringify({compilerOptions:{noLib:true,moduleDetection:"force"},files:["a.ts","globals.d.ts","module.d.ts"]}));
    writeFileSync(join(root,"a.ts"),"function run() {} run();");
    writeFileSync(join(root,"globals.d.ts"),"interface GlobalDoc { touch():void; }");
    writeFileSync(join(root,"module.d.ts"),"export interface PrivateDoc { touch():void; }");
    const report=capture(ts,root,"tsconfig.json",".");
    assert.equal(report.version,2);
    assert.equal(report.files.find(f=>f.index_path==="a.ts").source_scope,"Module");
    assert.equal(report.files.find(f=>f.index_path==="globals.d.ts").source_scope,"Syntax");
    assert.equal(report.files.find(f=>f.index_path==="module.d.ts").source_scope,"Module");
    assert.equal(report.calls.length,1); assert(report.calls[0].target);
  } finally { rmSync(root,{recursive:true,force:true}); }
});

test("compiler configuration, alias targets and support-only files retain distinct identities",()=>{
  const root=mkdtempSync(join(tmpdir(),"bw-project-oracle-"));
  try {
    writeFileSync(join(root,"base.json"),JSON.stringify({compilerOptions:{noLib:true,strict:true}}));
    writeFileSync(join(root,"tsconfig.json"),JSON.stringify({extends:"./base.json",files:["a.ts","support.d.ts"]}));
    writeFileSync(join(root,"support.d.ts"),"export declare function external():void;");
    writeFileSync(join(root,"a.ts"),"\uFEFFimport {external as renamed} from './support';\nfunction external() {}\n/*é🦀*/ renamed(); external();");
    const report=capture(ts,root,"tsconfig.json",".");
    assert.equal(report.calls.length,2);
    const support=report.files.find(f=>f.path.endsWith("/support.d.ts"));
    assert.deepEqual(report.source_binding_order.map(id=>report.files.find(f=>f.id===id).index_path), ["support.d.ts", "a.ts"]);
    assert.deepEqual(report.files.map(f=>f.index_path), ["a.ts", "support.d.ts"], "file IDs deliberately do not encode binding order");
    const source=report.files.find(f=>f.path.endsWith("/a.ts"));
    assert.equal(support.selected,false); assert.equal(source.selected,true);
    assert.equal(report.calls[0].target.file,support.id);
    assert.equal(report.calls[1].target.file,source.id);
    assert.equal(report.calls[0].site.byte_offset,Buffer.byteLength("\uFEFFimport {external as renamed} from './support';\nfunction external() {}\n/*é🦀*/ "));
    assert(report.inputs.some(i=>i.path.endsWith("/base.json")));
    assert.equal(report.compiler_options.strict,true);
  } finally { rmSync(root,{recursive:true,force:true}); }
});

test("project capture fingerprints configuration and keeps diagnostic calls unlabelled",()=>{
  const root=mkdtempSync(join(tmpdir(),"bw-project-oracle-"));
  try {
    writeFileSync(join(root,"tsconfig.json"),JSON.stringify({compilerOptions:{noLib:true},files:["a.ts"]}));
    writeFileSync(join(root,"a.ts"),"export function f() {} f(); missing();");
    const report=capture(ts,root,"tsconfig.json",".");
    assert.equal(report.calls.length,2); assert(report.calls[0].target);
    assert.equal(report.calls[1].reason,"no_compiler_target");
    assert(report.diagnostics.some(d=>d.code===2304));
    assert(report.inputs.some(i=>i.path.endsWith("/tsconfig.json")));
    assert.equal(report.selection.split,"development");
    assert.throws(()=>capture(ts,root,"tsconfig.json","../escape"),/escapes/);
  } finally { rmSync(root,{recursive:true,force:true}); }
});
