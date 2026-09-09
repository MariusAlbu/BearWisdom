// Independent compiler target verification, not a BearWisdom runtime dependency.
import assert from "node:assert/strict";
import { readFileSync, mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname, resolve, relative, isAbsolute } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { validateTargets, validateRejections } from "./rust_compiler_output.mjs";

export const PINNED_RUSTC = "rustc 1.94.0 (4a4ef493e 2026-03-02)";

export function stripMarkers(file, input, references, declarations) {
  let source = "", from = 0;
  for (const marker of input.matchAll(/\/\*@(ref|decl):(\d+)(?::(\w+))?\*\//g)) {
    source += input.slice(from, marker.index);
    const table = marker[1] === "ref" ? references : declarations, id = Number(marker[2]);
    assert(marker[1] !== "decl" || marker[3], "Declaration kind required");
    assert(marker[1] !== "ref" || !marker[3], "Reference marker cannot have a declaration kind");
    assert(Number.isSafeInteger(id) && id <= 0xffffffff, "Marker ID must fit u32");
    assert(!table.has(id), "Duplicate source marker");
    table.set(id, { file, byte: Buffer.byteLength(source), ...(marker[1] === "decl" ? { kind: marker[3] } : {}) });
    from = marker.index + marker[0].length;
  }
  source += input.slice(from);
  assert(!source.includes("/*@"), "Malformed oracle marker");
  return source;
}

function compiler(rustc, args, cwd, bootstrap = false) {
  const env = { ...process.env }; delete env.RUSTC_BOOTSTRAP;
  if (bootstrap) env.RUSTC_BOOTSTRAP = "bw_target_oracle";
  const result = spawnSync(rustc, args, { cwd, env, encoding: "utf8", windowsHide: true, timeout: 30000, maxBuffer: 64 * 1024 * 1024 });
  if (result.error) throw result.error;
  return result;
}

export function verify(cases, rustc = "rustc") {
  assert(cases.length, "Empty compiler cohort");
  const version = compiler(rustc, ["--version"]);
  assert.equal(version.status, 0, version.stderr);
  assert.equal(version.stdout.trim(), PINNED_RUSTC, "Unreviewed compiler debug format: update adapter tests before changing pin");
  let count = 0, negativeCount = 0;
  const names = new Set();
  for (const test of cases) {
    assert(!names.has(test.name), "Duplicate case name"); names.add(test.name);
    const root = mkdtempSync(join(tmpdir(), "bw-rust-target-oracle-"));
    try {
      const references = new Map(), declarations = new Map(), sources = new Map();
      for (const file of test.files) {
        const path = resolve(root, file.path), inside = relative(root, path);
        assert(inside && !inside.startsWith("..") && !isAbsolute(inside), "Fixture path escapes temporary workspace");
        assert(file.path === inside.replaceAll("\\", "/"), "Fixture path must be canonical and relative");
        assert(!sources.has(file.path), "Duplicate fixture file");
        const source = stripMarkers(file.path, file.source, references, declarations);
        sources.set(file.path, source); mkdirSync(dirname(path), { recursive: true }); writeFileSync(path, source);
      }
      assert(sources.has(test.entry), "Missing crate entry");
      const args = [test.entry, "--crate-name", "bw_target_oracle", "--crate-type", "lib", "--edition=2021", "--cap-lints=allow"];
      // Regular rustc must accept the fixture before debug-only evidence is used.
      const accepted = compiler(rustc, [...args, "--emit=metadata", "-o", "fixture.rmeta", "--error-format=json"], root);
      if (test.diagnostics) {
        assert.equal(accepted.status, 1, `${test.name}: expected ordinary compiler rejection`);
        negativeCount += validateRejections(test, references, accepted.stderr);
        continue;
      }
      assert.equal(accepted.status, 0, `${test.name}: compiler rejected source\n${accepted.stderr}`);
      const hir = compiler(rustc, [...args, "-Zunpretty=hir-tree"], root, true);
      const thir = compiler(rustc, [...args, "-Zunpretty=thir-tree"], root, true);
      assert.equal(hir.status, 0, hir.stderr); assert.equal(thir.status, 0, thir.stderr);
      count += validateTargets(test, references, declarations, hir.stdout, thir.stdout, sources);
    } finally {
      // root is the exact fresh mkdtemp result, never a supplied directory.
      rmSync(root, { recursive: true, force: true });
    }
  }
  return { compiler: PINNED_RUSTC, cases: cases.length, labelledCallsVerified: count, labelledRejectionsVerified: negativeCount };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const cases = ["rust_fixtures.json", "rust_receiver_fixtures.json", "rust_trait_fixtures.json", "rust_trait_selection_fixtures.json", "rust_qualified_fixtures.json", "rust_borrow_fixtures.json", "rust_argument_fixtures.json", "rust_local_value_fixtures.json", "rust_place_fixtures.json", "rust_pattern_fixtures.json"].flatMap(file => JSON.parse(readFileSync(new URL(file, import.meta.url), "utf8")));
  console.log(JSON.stringify(verify(cases, process.argv[2] || "rustc")));
}
