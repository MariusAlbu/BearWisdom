// =============================================================================
// ecosystem/jar_walker.rs — Java JAR + Android AAR archive extraction
//
// Maven/Gradle/Coursier projects pull thousands of transitive `.jar`
// artefacts. When the matching `-sources.jar` isn't downloaded
// (`mvn dependency:sources` / `gradle dependencies --refresh-dependencies`
// is opt-in), the symbol surface for those deps is invisible to the
// indexer. This walker fills that gap by parsing `.class` bytecode
// directly: every public/protected class, field, and method becomes a
// real `ParsedFile`/symbol entry that the resolver can bind against.
//
// Coverage: class + field + method names + JVM descriptors. Method bodies
// are not parsed (no call graph extraction from bytecode — too expensive
// for the per-edge resolution value). For a `.aar` (Android Archive),
// the inner `classes.jar` is extracted and walked the same way.
//
// **Implementation:** in-tree minimal class-file parser. The JVM class
// file format is stable and the subset we need (constant pool + class
// header + fields[] + methods[]) is small. Skipping attributes is
// straightforward — `u2 name_idx + u4 length + length bytes`.
// =============================================================================

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use crate::types::{ExtractedSymbol, ParsedFile, SymbolKind, Visibility, FlowMeta};

/// True for file extensions this walker knows how to crack open.
pub fn supports_extension(ext: &str) -> bool {
    matches!(ext, "jar" | "aar")
}

/// Open the archive at `path` and yield one `ParsedFile` per `.class`
/// member inside. The synthetic file path is
/// `ext:jar:<archive_path>!<member_path>` so chains differentiate per
/// class. Errors (bad zip, malformed bytecode) cause individual entries
/// to be skipped — never a hard failure.
pub fn walk_jar(path: &Path) -> Vec<ParsedFile> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let mut archive = match zip::ZipArchive::new(file) {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };
    let archive_str = path.to_string_lossy().replace('\\', "/");
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.is_dir() { continue }
        let name = entry.name().to_string();
        if !name.ends_with(".class") { continue }
        // Skip nested `META-INF/versions/*` multi-release duplicates —
        // they'd produce duplicate symbol entries.
        if name.starts_with("META-INF/versions/") { continue }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        if entry.read_to_end(&mut bytes).is_err() { continue }
        let Some(parsed) = parse_class_file(&bytes) else { continue };
        let virt_path = format!("ext:jar:{archive_str}!{name}");
        let pf = parsed_class_to_parsed_file(virt_path, parsed);
        out.push(pf);
    }
    // For .aar, extract classes.jar and walk it recursively.
    if path.extension().and_then(|e| e.to_str()) == Some("aar") {
        out.extend(walk_aar_inner_jar(path));
    }
    out
}

fn walk_aar_inner_jar(aar_path: &Path) -> Vec<ParsedFile> {
    let Ok(file) = File::open(aar_path) else { return Vec::new() };
    let Ok(mut archive) = zip::ZipArchive::new(file) else { return Vec::new() };
    let Ok(mut entry) = archive.by_name("classes.jar") else { return Vec::new() };
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    if entry.read_to_end(&mut bytes).is_err() { return Vec::new() }
    drop(entry);
    drop(archive);
    // Materialise to a temp file so we can re-use walk_jar's path-based path.
    let tmp = std::env::temp_dir().join(format!(
        "bw_aar_inner_{}.jar",
        std::process::id()
    ));
    if std::fs::write(&tmp, &bytes).is_err() { return Vec::new() }
    let result = walk_jar(&tmp);
    let _ = std::fs::remove_file(&tmp);
    result
}

// ---------------------------------------------------------------------------
// Minimal class-file parser
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ParsedClass {
    pub this_class: String,
    pub super_class: Option<String>,
    pub access_flags: u16,
    pub fields: Vec<ClassMember>,
    pub methods: Vec<ClassMember>,
}

#[derive(Debug)]
pub struct ClassMember {
    pub name: String,
    pub descriptor: String,
    pub access_flags: u16,
}

/// Parse a `.class` file. Returns `None` on any structural error —
/// callers skip the entry rather than aborting the whole walk.
pub fn parse_class_file(bytes: &[u8]) -> Option<ParsedClass> {
    let mut r = Reader::new(bytes);
    if r.u4()? != 0xCAFEBABE { return None }
    let _minor = r.u2()?;
    let _major = r.u2()?;
    let pool = parse_constant_pool(&mut r)?;
    let access_flags = r.u2()?;
    let this_class_idx = r.u2()? as usize;
    let super_class_idx = r.u2()? as usize;
    let this_class = pool.class_name(this_class_idx)?.to_string();
    let super_class = if super_class_idx == 0 {
        None
    } else {
        pool.class_name(super_class_idx).map(|s| s.to_string())
    };
    let interfaces_count = r.u2()? as usize;
    for _ in 0..interfaces_count { let _ = r.u2()?; }
    let fields = parse_member_list(&mut r, &pool)?;
    let methods = parse_member_list(&mut r, &pool)?;
    Some(ParsedClass { this_class, super_class, access_flags, fields, methods })
}

fn parse_member_list(r: &mut Reader, pool: &ConstantPool) -> Option<Vec<ClassMember>> {
    let count = r.u2()? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let access_flags = r.u2()?;
        let name_idx = r.u2()? as usize;
        let desc_idx = r.u2()? as usize;
        let name = pool.utf8(name_idx)?.to_string();
        let descriptor = pool.utf8(desc_idx)?.to_string();
        let attrs_count = r.u2()? as usize;
        for _ in 0..attrs_count {
            let _attr_name_idx = r.u2()?;
            let attr_len = r.u4()? as usize;
            r.skip(attr_len)?;
        }
        out.push(ClassMember { access_flags, name, descriptor });
    }
    Some(out)
}

struct ConstantPool {
    entries: Vec<CpEntry>,
}

enum CpEntry {
    Empty,
    Utf8(String),
    Class { name_idx: u16 },
    Other,
}

impl ConstantPool {
    fn utf8(&self, idx: usize) -> Option<&str> {
        match self.entries.get(idx)? {
            CpEntry::Utf8(s) => Some(s.as_str()),
            _ => None,
        }
    }
    fn class_name(&self, idx: usize) -> Option<&str> {
        match self.entries.get(idx)? {
            CpEntry::Class { name_idx } => self.utf8(*name_idx as usize),
            _ => None,
        }
    }
}

fn parse_constant_pool(r: &mut Reader) -> Option<ConstantPool> {
    let count = r.u2()? as usize;
    let mut entries: Vec<CpEntry> = Vec::with_capacity(count);
    entries.push(CpEntry::Empty); // cp[0] unused
    let mut i = 1;
    while i < count {
        let tag = r.u1()?;
        let (entry, occupies_two_slots) = match tag {
            1 => {
                // CONSTANT_Utf8
                let len = r.u2()? as usize;
                let bytes = r.bytes(len)?;
                let s = decode_utf8(bytes).unwrap_or_default();
                (CpEntry::Utf8(s), false)
            }
            7 => {
                // CONSTANT_Class
                let name_idx = r.u2()?;
                (CpEntry::Class { name_idx }, false)
            }
            // Two-slot entries (Long, Double).
            5 | 6 => {
                r.skip(8)?;
                (CpEntry::Other, true)
            }
            // Single u4 entries (Integer, Float, NameAndType, Fieldref, Methodref,
            // InterfaceMethodref, MethodHandle (4 bytes), Dynamic, InvokeDynamic).
            3 | 4 | 9 | 10 | 11 | 12 | 17 | 18 => {
                r.skip(4)?;
                (CpEntry::Other, false)
            }
            // String (u2), MethodType (u2), Module (u2), Package (u2).
            8 | 16 | 19 | 20 => {
                r.skip(2)?;
                (CpEntry::Other, false)
            }
            // MethodHandle: u1 reference_kind + u2 reference_index.
            15 => {
                r.skip(3)?;
                (CpEntry::Other, false)
            }
            _ => {
                // Unknown tag → bail.
                return None;
            }
        };
        entries.push(entry);
        if occupies_two_slots {
            entries.push(CpEntry::Empty);
            i += 2;
        } else {
            i += 1;
        }
    }
    Some(ConstantPool { entries })
}

fn decode_utf8(bytes: &[u8]) -> Option<String> {
    // Java uses modified UTF-8 (CESU-8 with NUL escaped). For names/descs
    // (which are ASCII in practice) plain UTF-8 decode works. Strict
    // CESU-8 handling could be added later if real names use 4-byte
    // codepoints (rare).
    std::str::from_utf8(bytes).ok().map(|s| s.to_string())
}

struct Reader<'a> { bytes: &'a [u8], pos: usize }

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Self { Self { bytes, pos: 0 } }
    fn u1(&mut self) -> Option<u8> {
        let b = *self.bytes.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }
    fn u2(&mut self) -> Option<u16> {
        let b = self.bytes.get(self.pos..self.pos + 2)?;
        self.pos += 2;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }
    fn u4(&mut self) -> Option<u32> {
        let b = self.bytes.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let b = self.bytes.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(b)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        if self.pos + n > self.bytes.len() { return None }
        self.pos += n;
        Some(())
    }
}

// ---------------------------------------------------------------------------
// Bytecode → ParsedFile bridge
// ---------------------------------------------------------------------------

const ACC_PUBLIC: u16 = 0x0001;
const ACC_PRIVATE: u16 = 0x0002;
const ACC_PROTECTED: u16 = 0x0004;
const ACC_STATIC: u16 = 0x0008;
const ACC_INTERFACE: u16 = 0x0200;
const ACC_ENUM: u16 = 0x4000;

fn parsed_class_to_parsed_file(virt_path: String, cls: ParsedClass) -> ParsedFile {
    let mut symbols: Vec<ExtractedSymbol> = Vec::new();
    let class_qname = jvm_name_to_dot(&cls.this_class);
    let class_short = jvm_short_name(&class_qname);
    let class_kind = if cls.access_flags & ACC_INTERFACE != 0 {
        SymbolKind::Interface
    } else if cls.access_flags & ACC_ENUM != 0 {
        SymbolKind::Enum
    } else {
        SymbolKind::Class
    };
    symbols.push(ExtractedSymbol {
        name: class_short.to_string(),
        qualified_name: class_qname.clone(),
        kind: class_kind,
        visibility: Some(visibility_for(cls.access_flags)),
        start_line: 0,
        end_line: 0,
        start_col: 0,
        end_col: 0,
        signature: cls.super_class.as_ref().map(|s| format!("extends {}", jvm_name_to_dot(s))),
        doc_comment: None,
        scope_path: None,
        parent_index: None,
        byte_offset: 0,
        declared_type: None,
        return_type: None,
        param_types: Vec::new(),
        generic_params: Vec::new(),
    });
    let parent_idx = 0usize;
    for field in &cls.fields {
        if !is_externally_visible(field.access_flags) { continue }
        symbols.push(ExtractedSymbol {
            name: field.name.clone(),
            qualified_name: format!("{class_qname}.{}", field.name),
            kind: SymbolKind::Field,
            visibility: Some(visibility_for(field.access_flags)),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(field.descriptor.clone()),
            doc_comment: None,
            scope_path: Some(class_qname.clone()),
            parent_index: Some(parent_idx),
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
    }
    for method in &cls.methods {
        if !is_externally_visible(method.access_flags) { continue }
        if method.name == "<clinit>" { continue }
        let kind = if method.name == "<init>" {
            SymbolKind::Constructor
        } else {
            SymbolKind::Method
        };
        symbols.push(ExtractedSymbol {
            name: method.name.clone(),
            qualified_name: format!("{class_qname}.{}", method.name),
            kind,
            visibility: Some(visibility_for(method.access_flags)),
            start_line: 0,
            end_line: 0,
            start_col: 0,
            end_col: 0,
            signature: Some(method.descriptor.clone()),
            doc_comment: None,
            scope_path: Some(class_qname.clone()),
            parent_index: Some(parent_idx),
            byte_offset: 0,
            declared_type: None,
            return_type: None,
            param_types: Vec::new(),
            generic_params: Vec::new(),
        });
    }
    ParsedFile {
        path: virt_path,
        language: "java".to_string(),
        content_hash: String::new(),
        size: 0,
        line_count: 0,
        mtime: None,
        package_id: None,
        content: None,
        has_errors: false,
        symbols,
        refs: Vec::new(),
        routes: Vec::new(),
        db_sets: Vec::new(),
        symbol_origin_languages: Vec::new(),
        ref_origin_languages: Vec::new(),
        symbol_from_snippet: Vec::new(),
        flow: FlowMeta::default(),
        demand_contributions: Vec::new(),
        alias_targets: Vec::new(),
        component_selectors: Vec::new(),
        plugin_flow_emissions: Vec::new(),
    }
}

fn is_externally_visible(access: u16) -> bool {
    // Public / protected only — private/package-private aren't part of
    // the resolvable surface for cross-jar refs.
    access & (ACC_PUBLIC | ACC_PROTECTED) != 0
}

fn visibility_for(access: u16) -> Visibility {
    if access & ACC_PUBLIC != 0 { Visibility::Public }
    else if access & ACC_PRIVATE != 0 { Visibility::Private }
    else if access & ACC_PROTECTED != 0 { Visibility::Protected }
    else { Visibility::Public }  // package-private treated as public for cross-jar refs
}

/// JVM internal class names use `/` separators; convert to `.`.
fn jvm_name_to_dot(name: &str) -> String { name.replace('/', ".") }

fn jvm_short_name(qname: &str) -> &str {
    qname.rsplit('.').next().unwrap_or(qname)
}

#[cfg(test)]
#[path = "jar_walker_tests.rs"]
mod tests;
