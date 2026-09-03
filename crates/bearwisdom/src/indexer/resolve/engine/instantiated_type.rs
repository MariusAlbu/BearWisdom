// =============================================================================
// engine/instantiated_type — the instance type a `new X(...)` initializer builds
// =============================================================================

use crate::indexer::resolve::engine::contract::{resolve_type_name_in_scope, SymbolLookup};
use crate::type_checker::core::types::{Type, TypeId};

use super::compilation::Compilation;

/// The instance type a `new X(...)` initializer builds. `X` may be a VALUE
/// in an enclosing scope — a `Ctor: typeof C` parameter — whose
/// constructor type's instance is what `new` yields; a value typed by a
/// bare instance name (the `typeof C` capture shape) carries that instance
/// directly. Only when no in-scope value carries the name does `X`
/// scope-resolve as the class itself. The global by-name pool is never
/// consulted for the value form — an unrelated package's same-named value
/// must not type this binding.
pub(super) fn instantiated_type(
    comp: &Compilation,
    name: &str,
    scope_path: Option<&str>,
    file: &str,
) -> TypeId {
    let mut scope = scope_path.unwrap_or("");
    while !scope.is_empty() {
        let qn = format!("{scope}.{name}");
        // Sibling packages repeat scope qnames (`useBaseQuery.Observer` in
        // four framework adapters) — the value in THIS file is the one the
        // initializer names, so a same-file candidate wins over the qname
        // slot's first-winner.
        let cands = comp.all_by_qualified_name(&qn);
        let s = cands
            .iter()
            .find(|s| &*s.file_path == file)
            .or_else(|| cands.iter().next());
        if let Some(s) = s {
            if matches!(
                s.kind.as_str(),
                "parameter" | "property" | "field" | "variable" | "constant"
            ) {
                let ft = comp
                    .type_info_by_id
                    .get(&s.id)
                    .and_then(|ti| ti.field_type_id)
                    .or_else(|| comp.type_info.get(&qn).and_then(|ti| ti.field_type_id));
                if let Some(ft) = ft {
                    let ft = comp.head_bind.bound(&comp.arena, comp, ft);
                    return match comp.arena.get(ft) {
                        Type::Constructor(inner) => inner,
                        _ => ft,
                    };
                }
            }
        }
        scope = match scope.rfind('.') {
            Some(i) => &scope[..i],
            None => "",
        };
    }
    let resolved = resolve_type_name_in_scope(name, scope_path, &comp.by_qname);
    comp.arena.class(&resolved)
}
