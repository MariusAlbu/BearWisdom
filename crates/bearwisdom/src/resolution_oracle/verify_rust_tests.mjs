// Real compiler adversarial checks. Nothing rewrites fixture labels or snapshots.
import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { verify } from "./verify_rust.mjs";

const cases = JSON.parse(readFileSync(new URL("./rust_fixtures.json", import.meta.url), "utf8"));
test("trait declaration binding is distinct from the overridden implementation body", () => {
  const fixture = {
    name: "trait_override_static_binding_target", entry: "lib.rs",
    files: [{ path: "lib.rs", source: "pub trait Save { /*@decl:1:method*/fn save(&self); } pub struct Doc; impl Save for Doc { /*@decl:2:method*/fn save(&self) {} } pub fn concrete(p: &Doc) { p./*@ref:11*/save(); } pub fn generic<T: Save>(p: &T) { p./*@ref:12*/save(); } pub fn qualified(p:&Doc) { <Doc as Save>::/*@ref:13*/save(p); }" }],
    labels: [[11, 1], [12, 1], [13, 1]],
  };
  assert.equal(verify([fixture]).labelledCallsVerified, 3);
  for (const label of fixture.labels) label[1] = 2;
  assert.throws(() => verify([fixture]), /wrong target label/);
});

test("real compiler rejects a deliberately swapped same-name target label", () => {
  const changed = structuredClone(cases[0]);
  changed.labels[0][1] = 2;
  assert.throws(() => verify([changed]), /wrong target label/);
});
test("real compiler diagnostics cannot attest a wrong error code", () => {
  const changed = structuredClone(cases.find(c => c.name === "wrong_receiver_cannot_borrow_a_member"));
  changed.diagnostics[0][1] = "E0425";
  assert.throws(() => verify([changed]), /missing\/ambiguous E0425/);
});
test("rejected source cannot be smuggled into positive target verification", () => {
  const changed = structuredClone(cases.find(c => c.name === "private_import_target_is_not_public"));
  delete changed.diagnostics;
  assert.throws(() => verify([changed]), /compiler rejected source/);
});
test("non-canonical and escaping source paths are rejected before writing", () => {
  for (const path of ["../outside.rs", "src/../lib.rs", "./lib.rs"]) {
    const changed = structuredClone(cases[0]); changed.files[0].path = path;
    assert.throws(() => verify([changed]), /Fixture path/);
  }
});
