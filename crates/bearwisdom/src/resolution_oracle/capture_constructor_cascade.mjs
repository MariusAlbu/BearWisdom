// Supplement an immutable call-navigation cohort with independently selected
// constructor/call signatures and compiler binding order for one source function.
// Usage: node ... <typescript-module> <manifest> <index-path> <function> <new-output>
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { resolve } from 'node:path';
import assert from 'node:assert/strict';
const ts = createRequire(import.meta.url)(process.argv[2]);
assert.equal(ts.version, '5.9.3');
const bytes = readFileSync(process.argv[3]), manifest = JSON.parse(bytes);
assert.equal(manifest.compiler.version, ts.version);
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const normalize = path => resolve(path).replaceAll('\\', '/');
const inputs = new Map([...manifest.inputs, ...manifest.files].map(f => [normalize(f.path), f.sha256]));
const verify = () => { for (const [path, hash] of inputs) assert.equal(digest(readFileSync(path)), hash, path); };
verify();
const readFile = path => {
  try {
    const bytes = readFileSync(path), key = normalize(path);
    assert(inputs.has(key), `Unrecorded compiler input: ${key}`);
    assert.equal(digest(bytes), inputs.get(key), key);
    return bytes.toString('utf8');
  } catch (error) { if (['ENOENT', 'ENOTDIR'].includes(error.code)) return undefined; throw error; }
};
const errors = [];
const parsed = ts.getParsedCommandLineOfConfigFile(manifest.config, {}, { ...ts.sys, readFile, onUnRecoverableConfigFileDiagnostic: d => errors.push(d) });
assert(parsed && !errors.length);
assert.deepEqual(parsed.options, manifest.compiler_options);
const host = ts.createCompilerHost(parsed.options, true); host.readFile = readFile;
host.writeFile = () => { throw new Error('Oracle must not emit'); };
const program = ts.createProgram({ rootNames: parsed.fileNames, options: parsed.options, projectReferences: parsed.projectReferences, host });
assert.deepEqual([...parsed.errors, ...ts.getPreEmitDiagnostics(program)], []);
const files = new Map(manifest.files.map(f => [normalize(f.path), f]));
const order = program.getSourceFiles().map(sf => {
  const file = files.get(normalize(sf.fileName)); assert(file, sf.fileName);
  assert.equal(digest(Buffer.from(sf.text)), file.sha256);
  return file.id;
});
assert.equal(new Set(order).size, manifest.files.length);
const target = manifest.files.find(f => f.index_path === process.argv[4]); assert(target);
const source = program.getSourceFile(target.path); assert(source);
const functions = source.statements.filter(node => ts.isFunctionDeclaration(node) && node.name?.text === process.argv[5]);
assert.equal(functions.length, 1);
const span = node => {
  const sf = node.getSourceFile(), file = files.get(normalize(sf.fileName)); assert(file);
  return { file: file.id, start: Buffer.byteLength(sf.text.slice(0, node.getStart(sf))), end: Buffer.byteLength(sf.text.slice(0, node.getEnd())) };
};
const checker = program.getTypeChecker(), signatures = [], callbacks = [];
const visit = node => {
  if (ts.isNewExpression(node) || ts.isCallExpression(node)) {
    const selected = checker.getResolvedSignature(node)?.declaration; assert(selected);
    signatures.push({ site: span(node), kind: ts.isNewExpression(node) ? 'construct' : 'call',
      selected: span(selected), selected_kind: ts.SyntaxKind[selected.kind], return_type: checker.typeToString(checker.getTypeAtLocation(node)) });
  }
  if (ts.isArrowFunction(node)) {
    const signature = checker.getSignatureFromDeclaration(node); assert(signature);
    const predicate = checker.getTypePredicateOfSignature(signature);
    callbacks.push({ site: span(node), return_type: checker.typeToString(checker.getReturnTypeOfSignature(signature)),
      predicate: predicate ? { parameter: predicate.parameterIndex, type: predicate.type ? checker.typeToString(predicate.type) : null } : null });
  }
  ts.forEachChild(node, visit);
};
visit(functions[0]); verify();
assert(signatures.some(s => s.kind === 'construct') && callbacks.length);
const result = { version: 1, compiler: manifest.compiler, manifest_sha256: digest(bytes), source_binding_order: order,
  selection: span(functions[0]), signatures, callbacks,
  limitations: ['supplemental_development_cohort', 'display_types_are_diagnostics_not_engine_inputs', 'not_complete_filesystem_resolution_fingerprint'] };
writeFileSync(process.argv[6], JSON.stringify(result, null, 2) + '\n', { flag: 'wx' });
console.log(JSON.stringify({ sources: order.length, signatures, callbacks }));
