//! TypeScript external declaration-module augmentation recognition.
//!
//! This module owns the declaration spelling, quoted module syntax, external
//! virtual-path package convention, and the source scan used when the external
//! parse cache has discarded source text.

use crate::languages::ModuleAugmentation;

/// Collect the cross-package interface augmentations declared by one external
/// TypeScript source file. A package augmenting its own module is an ordinary
/// in-package declaration and does not need a graft record.
pub(crate) fn collect(source: &str, virtual_path: &str) -> Vec<ModuleAugmentation> {
    if !source.contains("declare module") {
        return Vec::new();
    }
    let Some(package) = package_from_virtual_path(virtual_path) else {
        return Vec::new();
    };
    scan(source)
        .into_iter()
        .filter(|(module, _)| module != package)
        .map(|(module, interface)| ModuleAugmentation {
            augmenting_qname: format!("{package}.{interface}"),
            module,
            interface,
        })
        .collect()
}

/// Scan source for quoted declaration-module interface blocks. An unquoted
/// declaration module is a namespace and therefore not an augmentation.
pub(crate) fn scan(source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut current: Option<String> = None;
    let mut depth: i32 = 0;
    for line in source.lines() {
        let line = line.trim();
        match &current {
            None => {
                if let Some(rest) = line.strip_prefix("declare module ") {
                    if let Some(module) = first_quoted(rest.trim_start()) {
                        let module = module.to_string();
                        if let Some((_, body)) = line.split_once('{') {
                            if let Some(interface) = interface_name(body.trim()) {
                                out.push((module.clone(), interface.to_string()));
                            }
                        }
                        current = Some(module);
                        depth = brace_delta(line);
                        if depth <= 0 {
                            current = None;
                        }
                    }
                }
            }
            Some(module) => {
                if let Some(interface) = interface_name(line) {
                    out.push((module.clone(), interface.to_string()));
                }
                depth += brace_delta(line);
                if depth <= 0 {
                    current = None;
                }
            }
        }
    }
    out
}

/// Extract the package component of a TypeScript external virtual path.
fn package_from_virtual_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("ext:ts:")?;
    if rest.starts_with('@') {
        let mut parts = rest.splitn(3, '/');
        let scope = parts.next()?;
        let name = parts.next()?;
        Some(&rest[..scope.len() + 1 + name.len()])
    } else {
        Some(&rest[..rest.find('/')?])
    }
}

pub(crate) fn external_reexport_target_qname(
    virtual_path: &str,
    target_name: &str,
) -> Option<String> {
    Some(format!(
        "{}.{}",
        package_from_virtual_path(virtual_path)?,
        target_name
    ))
}

fn first_quoted(value: &str) -> Option<&str> {
    let quote = value
        .chars()
        .next()
        .filter(|quote| matches!(quote, '\'' | '"'))?;
    let rest = &value[quote.len_utf8()..];
    rest.find(quote).map(|end| &rest[..end])
}

fn brace_delta(line: &str) -> i32 {
    line.matches('{').count() as i32 - line.matches('}').count() as i32
}

pub(crate) fn interface_name(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("export interface ")
        .or_else(|| line.strip_prefix("interface "))?;
    let name = rest
        .split(|character: char| character == '<' || character == ' ' || character == '{')
        .next()?
        .trim();
    (!name.is_empty()).then_some(name)
}
