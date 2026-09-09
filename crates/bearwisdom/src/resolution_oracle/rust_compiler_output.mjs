// Test-only rustc debug-dump adapter. Numeric DefIds join evidence, never names.
import assert from "node:assert/strict";

export function sourceSpan(text, sources) {
  const match = /^(.+):(\d+):(\d+): (\d+):(\d+) \(#(\d+)\)$/.exec(text);
  assert(match, `Unsupported compiler span: ${text}`);
  const [, compilerFile, sl, sc, el, ec, context] = match;
  const file = compilerFile.replaceAll("\\", "/");
  assert.equal(context, "0", "Macro-expanded source needs a separate identity policy");
  const source = sources.get(file);
  assert.notEqual(source, undefined, `Compiler span outside fixture: ${file}`);
  const byte = (line, column) => {
    const lines = source.split("\n"), row = Number(line) - 1, col = Number(column) - 1;
    assert(row >= 0 && row < lines.length, "Compiler line outside source");
    const characters = [...lines[row]];
    assert(col >= 0 && col <= characters.length, "Compiler column outside source");
    return Buffer.byteLength(lines.slice(0, row).join("\n") + (row ? "\n" : "") + characters.slice(0, col).join(""));
  };
  const span = { file, start: byte(sl, sc), end: byte(el, ec) };
  assert(span.end >= span.start, "Reversed compiler span");
  return span;
}

export function declarationSpans(hir, sources) {
  const owners = [...hir.matchAll(/^DefId\((\d+):(\d+) ~ [^\n]+\) => OwnerNodes \{/gm)];
  assert(owners.length, "No HIR owner evidence");
  const declarations = new Map();
  for (let i = 0; i < owners.length; i++) {
    const owner = owners[i];
    const body = hir.slice(owner.index, owners[i + 1]?.index ?? hir.length).split("\n    parents:")[0];
    const category = /^        node: (Item|ImplItem|TraitItem)\(/m.exec(body)?.[1];
    if (!category || !/^                kind: Fn[ ({]/m.test(body)) continue;
    const spans = [...body.matchAll(/^                span: (.+),$/gm)];
    assert.equal(spans.length, 1, "Expected exactly one function declaration span");
    const id = `${owner[1]}:${owner[2]}`;
    assert(!declarations.has(id), "Duplicate compiler declaration ID");
    declarations.set(id, { ...sourceSpan(spans[0][1], sources), kind: category === "Item" ? "function" : "method" });
  }
  return declarations;
}

export function directCalls(thir, sources) {
  const lines = thir.split("\n"), calls = [];
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].trim() !== "Call {") continue;
    const indentation = lines[i].match(/^ */)[0].length;
    const head = [];
    for (let j = i + 1; j < lines.length; j++) {
      if (lines[j].trim() && lines[j].match(/^ */)[0].length <= indentation) break;
      head.push(lines[j]);
      if (lines[j].trim() === "args: [") break;
    }
    const direct = head[0]?.trim().match(/^ty: FnDef\(DefId\((\d+):(\d+) ~ /);
    if (!direct) continue; // Indirect calls need local-binding, not FnDef evidence.
    const sourceCall = head.find(line => line.trim().startsWith("from_hir_call:"));
    assert(sourceCall, "Missing compiler call-origin evidence");
    if (sourceCall.trim() !== "from_hir_call: true") continue;
    const fun = head.findIndex(line => line.trim() === "fun:");
    assert(fun >= 0, "Missing compiler callee expression");
    const spans = head.slice(fun + 1).filter(line => line.match(/^ */)[0].length === indentation + 12 && line.trim().startsWith("span: "));
    assert.equal(spans.length, 1, "Ambiguous or missing compiler callee span");
    calls.push({ target: `${direct[1]}:${direct[2]}`, ...sourceSpan(spans[0].trim().slice(6), sources) });
  }
  return calls;
}

export function validateTargets(test, references, declarations, hir, thir, sources) {
  const targets = declarationSpans(hir, sources), calls = directCalls(thir, sources);
  assert.equal(test.labels.length, references.size, "Every reference needs one label");
  const checked = new Set(), consumed = new Set();
  for (const [reference, target] of test.labels) {
    assert(!checked.has(reference), "Duplicate reference label"); checked.add(reference);
    const ref = references.get(reference), expected = declarations.get(target);
    assert(ref && expected && target !== null, "Unknown reference/declaration or unsupported negative label");
    const matches = calls.filter(call => call.file === ref.file && call.start <= ref.byte && ref.byte < call.end);
    assert.equal(matches.length, 1, `${test.name} reference ${reference}: missing/ambiguous compiler call`);
    assert(!consumed.has(matches[0]), "Multiple reference markers claim the same compiler call"); consumed.add(matches[0]);
    const actual = targets.get(matches[0].target);
    assert(actual, `Compiler target ${matches[0].target} has no local declaration span`);
    assert.deepEqual({ file: actual.file, byte: actual.start, kind: actual.kind }, expected, `${test.name} reference ${reference}: wrong target label`);
  }
  return checked.size;
}

// A rejected fixture cannot supply positive target evidence. Negative labels
// require the authored compiler error at the exact marked occurrence instead.
export function validateRejections(test, references, stderr) {
  assert.equal(test.labels.length, references.size, "Every reference needs one label");
  assert.equal(test.diagnostics.length, references.size, "Every negative needs diagnostic evidence");
  const diagnostics = stderr.split(/\r?\n/).filter(line => line.trim()).map(line => JSON.parse(line));
  const codes = new Map(test.diagnostics), checked = new Set();
  assert.equal(codes.size, test.diagnostics.length, "Duplicate diagnostic label");
  for (const [reference, target] of test.labels) {
    assert.equal(target, null, "Rejected source cannot attest positive call targets");
    assert(!checked.has(reference), "Duplicate reference label"); checked.add(reference);
    const ref = references.get(reference), code = codes.get(reference);
    assert(ref && code, "Missing negative occurrence or diagnostic code");
    const matches = diagnostics.filter(diagnostic => diagnostic.level === "error" && diagnostic.code?.code === code
      && diagnostic.spans.some(span => span.is_primary && span.file_name.replaceAll("\\", "/") === ref.file
        && span.byte_start <= ref.byte && ref.byte < span.byte_end));
    assert.equal(matches.length, 1, `${test.name} reference ${reference}: missing/ambiguous ${code} evidence`);
  }
  return checked.size;
}
