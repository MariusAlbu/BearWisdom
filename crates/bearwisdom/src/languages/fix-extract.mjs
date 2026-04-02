import fs from "fs";

function fixExtract(file, structName) {
  let c = fs.readFileSync(file, "utf8");

  // Remove mod declarations at top
  c = c.replace(/^mod \w+;\n/gm, "");
  c = c.replace(/^pub\(super\) mod \w+;\n/gm, "");

  // Remove test blocks at bottom
  c = c.replace(
    /\n#\[cfg\(test\)\]\n#\[path = "[^"]+"\]\nmod tests;\n?/g,
    "\n",
  );
  c = c.replace(/\n#\[cfg\(test\)\]\nmod tests;\n?/g, "\n");

  // Add ExtractionResult import
  if (!c.includes("ExtractionResult")) {
    const insertBefore = c.indexOf("use crate::types");
    if (insertBefore >= 0) {
      c =
        c.slice(0, insertBefore) +
        "use crate::parser::extractors::ExtractionResult;\n" +
        c.slice(insertBefore);
    }
  }

  // Remove struct definition
  const structStart = c.indexOf("pub struct " + structName + " {");
  if (structStart >= 0) {
    let depth = 0;
    let end = structStart;
    for (let i = structStart; i < c.length; i++) {
      if (c[i] === "{") depth++;
      if (c[i] === "}") {
        depth--;
        if (depth === 0) {
          end = i + 1;
          break;
        }
      }
    }
    c = c.slice(0, structStart) + c.slice(end);
  }

  // Replace return type
  c = c.split("-> " + structName).join("-> ExtractionResult");

  // Replace all struct name usages in struct literals
  c = c.split(structName + " {").join("ExtractionResult {");

  // Fix simple returns: ExtractionResult { symbols, refs, has_errors }
  c = c.replace(
    /ExtractionResult \{ symbols, refs, has_errors \}/g,
    "ExtractionResult::new(symbols, refs, has_errors)",
  );

  // Fix error returns - add routes/db_sets
  c = c.replace(
    /ExtractionResult \{\n(\s+)symbols: vec!\[\],\n\s+refs: vec!\[\],\n\s+has_errors: true,?\n\s+\}/g,
    (match, indent) =>
      `ExtractionResult {\n${indent}symbols: vec![],\n${indent}refs: vec![],\n${indent}routes: vec![],\n${indent}db_sets: vec![],\n${indent}has_errors: true,\n${indent.slice(4) || indent}}`,
  );

  // Make scope kinds public
  c = c.replace(/^static (\w+_SCOPE_KINDS)/gm, "pub(crate) static $1");

  // Fix emit_chain_type_ref paths
  c = c.split("super::emit_chain_type_ref").join("crate::parser::extractors::emit_chain_type_ref");

  fs.writeFileSync(file, c);
  console.log("Fixed " + file);
}

fixExtract("java/extract.rs", "JavaExtraction");
fixExtract("python/extract.rs", "PythonExtraction");
fixExtract("rust_lang/extract.rs", "RustExtraction");
