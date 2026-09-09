// Compiler-only source identity. No engine output is available to this module.
import assert from "node:assert/strict";

export function byteAt(sf, position) {
  assert(Number.isInteger(position) && position >= 0 && position <= sf.text.length);
  return Buffer.byteLength(sf.text.slice(0, position), "utf8");
}

export function declarationSite(ts, declaration, files) {
  const sf = declaration.getSourceFile(), file = files.get(sf.fileName);
  if (!file) return { reason: "target_outside_compiler_manifest" };
  let kind;
  if (ts.isFunctionDeclaration(declaration) || ts.isFunctionExpression(declaration)) kind = "function";
  else if (ts.isMethodDeclaration(declaration) || ts.isMethodSignature(declaration)) kind = "method";
  else if (ts.isPropertyDeclaration(declaration) || ts.isPropertySignature(declaration)) kind = "property";
  else if (ts.isVariableDeclaration(declaration) && ts.isIdentifier(declaration.name)) {
    const init = declaration.initializer;
    kind = init && (ts.isArrowFunction(init) || (ts.isFunctionExpression(init) && !init.name)) ? "function" : "variable";
  } else return { reason: "unsupported_declaration_form", declaration_kind: ts.SyntaxKind[declaration.kind] };
  // Export/default/ambient wrappers are outside the concrete declaration in
  // the source-address contract. Other modifiers (async, public, static) remain.
  let start = declaration.getStart(sf);
  const scanner = ts.createScanner(ts.ScriptTarget.Latest, true, sf.languageVariant, sf.text, undefined, start);
  let token = scanner.scan();
  while ([ts.SyntaxKind.ExportKeyword, ts.SyntaxKind.DefaultKeyword, ts.SyntaxKind.DeclareKeyword].includes(token)) token = scanner.scan();
  start = scanner.getTokenPos();
  const position = sf.getLineAndCharacterOfPosition(start);
  const lineStart = sf.getPositionOfLineAndCharacter(position.line, 0);
  return { target: { file: file.id, line: position.line, col: byteAt(sf, start) - byteAt(sf, lineStart), kind } };
}

export function captureCalls(ts, program, files) {
  const checker = program.getTypeChecker(), calls = [], sites = new Set();
  for (const sf of program.getSourceFiles()) {
    const file = files.get(sf.fileName);
    if (!file?.selected) continue;
    function visit(node) {
      if (ts.isCallExpression(node)) {
        const expression = node.expression;
        const selector = ts.isPropertyAccessExpression(expression) ? expression.name : expression;
        const site = { file: file.id, byte_offset: byteAt(sf, selector.getStart(sf)), kind: "calls" };
        const key = JSON.stringify(site);
        assert(!sites.has(key), "Ambiguous compiler call address: " + key); sites.add(key);
        let label;
        if (!ts.isIdentifier(selector) && !ts.isPrivateIdentifier(selector)) label = { reason: "unsupported_call_form", expression_kind: ts.SyntaxKind[expression.kind] };
        else {
          let symbol = checker.getSymbolAtLocation(selector);
          if (symbol && (symbol.flags & ts.SymbolFlags.Alias)) symbol = checker.getAliasedSymbol(symbol);
          const declarations = symbol?.declarations || [];
          if (declarations.length !== 1) label = { reason: declarations.length ? "multiple_declarations" : "no_compiler_target", declaration_count: declarations.length };
          else label = declarationSite(ts, declarations[0], files);
        }
        calls.push({ site, ...label });
      }
      ts.forEachChild(node, visit);
    }
    visit(sf);
  }
  return calls.sort((a, b) => a.site.file - b.site.file || a.site.byte_offset - b.site.byte_offset);
}
