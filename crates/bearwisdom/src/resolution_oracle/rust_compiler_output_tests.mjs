import assert from "node:assert/strict";
import test from "node:test";
import { sourceSpan, declarationSpans, directCalls, validateTargets, validateRejections } from "./rust_compiler_output.mjs";
import { stripMarkers } from "./verify_rust.mjs";

const sources = new Map([["lib.rs", "fn a() {} fn a() {} a();"]]);
const span = (start, end) => `lib.rs:1:${start + 1}: 1:${end + 1} (#0)`;
const owner = (id, start, end) => `DefId(0:${id} ~ printed::same::name) => OwnerNodes {
    node: ParentedNode {
        node: ImplItem(
            ImplItem {
                kind: Fn(
                span: ${span(start, end)},
            },
        ),
    },
    parents: [],
}`;
const call = (id, start = 20, end = 21) => `        Call {
            ty: FnDef(DefId(0:${id} ~ printed::same::name), [])
            from_hir_call: true
            fn_span: ${span(start, end + 2)}
            fun:
                Expr {
                    span: ${span(start, end)}
                }
            args: [
            ]
        }`;
const hir = owner(6, 0, 9) + "\n" + owner(7, 10, 19);
const refs = new Map([[11, { file: "lib.rs", byte: 20 }]]);
const decls = new Map([[1, { file: "lib.rs", byte: 0, kind: "method" }], [2, { file: "lib.rs", byte: 10, kind: "method" }]]);
const fixture = { name: "namesake", labels: [[11, 2]] };

test("equal printed names cannot merge different compiler declaration IDs", () => {
  assert.equal(declarationSpans(hir, sources).size, 2);
  assert.equal(validateTargets(fixture, refs, decls, hir, call(7), sources), 1);
  assert.throws(() => validateTargets(fixture, refs, decls, hir, call(6), sources), /wrong target label/);
  assert.throws(() => validateTargets({ ...fixture, labels: [[11, 1]] }, refs, decls, hir, call(7), sources), /wrong target label/);
  const wrongKind = new Map(decls); wrongKind.set(2, { ...decls.get(2), kind: "function" });
  assert.throws(() => validateTargets(fixture, refs, wrongKind, hir, call(7), sources), /wrong target label/);
});

test("Unicode compiler character columns become UTF-8 byte positions", () => {
  const source = "// 🦀é\nαβ();";
  assert.deepEqual(sourceSpan("lib.rs:2:1: 2:3 (#0)", new Map([["lib.rs", source]])),
    { file: "lib.rs", start: Buffer.byteLength("// 🦀é\n"), end: Buffer.byteLength("// 🦀é\nαβ") });
});

test("span evidence fails closed for macros, wrong files, bounds and format", () => {
  for (const invalid of ["lib.rs:1:1: 1:2 (#1)", "other.rs:1:1: 1:2 (#0)",
    "lib.rs:2:1: 2:2 (#0)", "lib.rs:1:1: 1:999 (#0)", "lib.rs:1:3: 1:1 (#0)", "unreviewed format"]) {
    assert.throws(() => sourceSpan(invalid, sources));
  }
});

test("missing, duplicate and indirect call evidence never chooses a first match", () => {
  for (const invalid of ["", call(7) + "\n" + call(7), call(7).replace("FnDef(DefId(0:7 ~ printed::same::name), [])", "FnPtr(...)"),
    call(7).replace("from_hir_call: true", "from_hir_call: false")]) {
    assert.throws(() => validateTargets(fixture, refs, decls, hir, invalid, sources), /missing\/ambiguous compiler call/);
  }
  assert.throws(() => directCalls(call(7).replace("from_hir_call:", "changed_origin:"), sources), /origin evidence/);
  assert.throws(() => declarationSpans(hir + "\n" + owner(7, 10, 19), sources), /Duplicate compiler declaration/);
  assert.throws(() => validateTargets(fixture, refs, decls, hir, call(99), sources), /no local declaration span/);
});

test("labels cannot duplicate or omit physical reference evidence", () => {
  assert.throws(() => validateTargets({ ...fixture, labels: [] }, refs, decls, hir, call(7), sources), /Every reference/);
  const repeatedRefs = new Map([...refs, [12, refs.get(11)]]);
  assert.throws(() => validateTargets({ ...fixture, labels: [[11, 2], [11, 2]] }, repeatedRefs, decls, hir, call(7), sources), /Duplicate reference/);
  assert.throws(() => validateTargets({ ...fixture, labels: [[11, 2], [12, 2]] }, repeatedRefs, decls, hir, call(7), sources), /same compiler call/);
  assert.throws(() => validateTargets({ ...fixture, labels: [[11, null]] }, refs, decls, hir, call(7), sources), /unsupported negative/);
});

test("source marker parsing agrees with the Rust u32 and byte-offset contract", () => {
  const references = new Map(), declarations = new Map();
  assert.equal(stripMarkers("lib.rs", "é/*@decl:1:function*/fn a() {} /*@ref:11*/a();", references, declarations), "éfn a() {} a();");
  assert.equal(declarations.get(1).byte, 2);
  assert.equal(references.get(11).byte, Buffer.byteLength("éfn a() {} "));
  for (const invalid of ["/*@ref:1", "/*@ref:1*/a(); /*@ref:1*/b();", "/*@decl:1*/", "/*@wat:1*/", "/*@ref:1:method*/", "/*@ref:4294967296*/"]) {
    assert.throws(() => stripMarkers("lib.rs", invalid, new Map(), new Map()));
  }
});

const rejected = { name: "negative", labels: [[11, null]], diagnostics: [[11, "E0425"]] };
const diagnostic = { level: "error", code: { code: "E0425" }, spans: [{ file_name: "lib.rs", byte_start: 20, byte_end: 21, is_primary: true }] };
test("negative labels need the exact compiler code at the marked primary span", () => {
  assert.equal(validateRejections(rejected, refs, JSON.stringify(diagnostic)), 1);
  for (const changed of [{ ...diagnostic, level: "warning" }, { ...diagnostic, code: { code: "E0599" } },
    { ...diagnostic, spans: [{ ...diagnostic.spans[0], is_primary: false }] },
    { ...diagnostic, spans: [{ ...diagnostic.spans[0], byte_start: 21 }] },
    { ...diagnostic, spans: [{ ...diagnostic.spans[0], file_name: "other.rs" }] }]) {
    assert.throws(() => validateRejections(rejected, refs, JSON.stringify(changed)), /missing\/ambiguous/);
  }
  assert.throws(() => validateRejections(rejected, refs, [diagnostic, diagnostic].map(d => JSON.stringify(d)).join("\n")), /missing\/ambiguous/);
  assert.throws(() => validateRejections({ ...rejected, labels: [[11, 2]] }, refs, JSON.stringify(diagnostic)), /cannot attest positive/);
});
