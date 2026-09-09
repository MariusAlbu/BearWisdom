// Developer-only code generation. Cargo builds exclusively from vendored output.
import { copyFileSync, mkdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';

const root = dirname(fileURLToPath(import.meta.url));
const upstream = join(root, 'node_modules/tree-sitter-typescript');
const dependencies = JSON.parse(readFileSync(join(root, 'package.json'))).devDependencies;
for (const [name, version] of Object.entries(dependencies)) {
  const actual = JSON.parse(readFileSync(join(root, 'node_modules', name, 'package.json'))).version;
  if (actual !== version) throw new Error(`${name}: expected ${version}, found ${actual}`);
}
const cli = join(root, 'node_modules/tree-sitter-cli', process.platform === 'win32' ? 'tree-sitter.exe' : 'tree-sitter');
const run = (...args) => {
  const result = spawnSync(cli, args, { cwd: root, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`tree-sitter failed: ${result.status}`);
};
run('--version');
for (const dialect of ['typescript', 'tsx']) {
  // Explicit ABI avoids metadata-dependent generation and retains runtime 0.25.
  run('generate', `${dialect}/grammar.js`, '--abi', '14', '--output', `${dialect}/src`);
  const header = readFileSync(join(root, dialect, 'src/parser.c'), 'utf8').slice(0, 200);
  if (!header.includes(`tree-sitter v${dependencies['tree-sitter-cli']}`)) {
    throw new Error(`Unexpected generator version in ${dialect}/src/parser.c`);
  }
}
for (const relative of ['common/scanner.h', 'typescript/src/scanner.c', 'tsx/src/scanner.c',
  'queries/highlights.scm', 'queries/locals.scm', 'queries/tags.scm', 'LICENSE']) {
  const destination = join(root, relative);
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(join(upstream, relative), destination);
}
copyFileSync(join(root, 'node_modules/tree-sitter-javascript/LICENSE'), join(root, 'LICENSE.javascript'));
