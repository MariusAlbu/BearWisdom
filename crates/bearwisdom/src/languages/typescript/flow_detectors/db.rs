// =============================================================================
// languages/typescript/flow_detectors/db.rs — DB query / ORM detectors
//
// Recognises DbQuery FlowEmissions from TypeScript ORM call chains.
// Supported shapes:
//
//   * Prisma — `prisma.user.findMany()` / `prisma.\([...])`
//   * TypeORM — `userRepository.find()` / `manager.find(User, ...)`
//   * Mongoose — `User.find()` / `user.save()`
//   * Sequelize — `User.findOne()` / `user.update()` / `user.destroy()`
//
// Entity name inference handles repository suffixes (`UsersRepository` →
// `User`), pluralization, and case normalisation.
// =============================================================================

use crate::indexer::resolve::engine::{FileContext, ImportEntry, RefContext, SymbolLookup};
use crate::indexer::resolve::flow_emit::{DbQueryOp, FlowEmission};

use super::first_arg_string;

pub(crate) fn detect_db_query_emission(
    chain: &crate::types::MemberChain,
) -> Option<FlowEmission> {
    // Production path runs through the imports-aware variant; this no-imports
    // wrapper exists for unit tests that don't need the gating semantics.
    detect_db_query_emission_inner(chain, true)
}

/// Imports-aware variant of the ORM-call detector.
///
/// The Mongoose/Sequelize PascalCase branch is the noisiest — `Object.create`,
/// `Headers.find`, `URL.create`, `Array.from`-like shapes all match. Gating
/// on the file actually importing one of those ORMs eliminates the bulk of
/// false positives without losing any real query. The Prisma + TypeORM
/// branches already have stronger structural signals (3-segment chain with
/// Prisma-specific op set; declared `Repository<>` type or `Repository`
/// suffix) and stay open even without an explicit import — Prisma clients
/// are commonly long-lived re-exported instances whose import trail can't be
/// recovered from the call site alone.
pub(crate) fn detect_db_query_emission_with_imports(
    chain: &crate::types::MemberChain,
    file_imports: &[ImportEntry],
) -> Option<FlowEmission> {
    detect_db_query_emission_inner(chain, file_imports_mongoose_or_sequelize(file_imports))
}

fn detect_db_query_emission_inner(
    chain: &crate::types::MemberChain,
    mongoose_seq_branch_enabled: bool,
) -> Option<FlowEmission> {
    let segs = &chain.segments;
    if segs.is_empty() {
        return None;
    }

    // Prisma: <client>.<camelModel>.<prismaOp>
    if segs.len() >= 3 {
        let model_seg = &segs[1];
        let op_seg = &segs[2];
        if is_camel_case_first(&model_seg.name) && is_prisma_op(&op_seg.name) {
            return Some(FlowEmission::DbQuery {
                entity_name: capitalize_first(&model_seg.name),
                operation: classify_orm_op(&op_seg.name),
            });
        }
    }

    // TypeORM repository — declared-type or name-suffix signal on the root.
    if segs.len() >= 2 {
        let root = &segs[0];
        let op_seg = &segs[1];
        if is_typeorm_op(&op_seg.name) {
            if let Some(dt) = root.declared_type.as_deref() {
                let dt_root = dt.split('<').next().unwrap_or(dt);
                if matches!(dt_root, "Repository" | "TreeRepository" | "MongoRepository") {
                    if let Some(entity) = root.type_args.first() {
                        return Some(FlowEmission::DbQuery {
                            entity_name: entity.clone(),
                            operation: classify_orm_op(&op_seg.name),
                        });
                    }
                }
            }
            if let Some(entity) = repository_suffix_entity(&root.name) {
                return Some(FlowEmission::DbQuery {
                    entity_name: entity,
                    operation: classify_orm_op(&op_seg.name),
                });
            }
        }
    }

    // Mongoose / Sequelize: <PascalModel>.<staticOp> — gated by the caller.
    // Without the gate, `Object.create`, `Headers.find`, `URL.create`, and
    // similar JS-builtin call shapes all match. The imports-aware variant
    // enables this branch only when mongoose/sequelize is imported.
    if mongoose_seq_branch_enabled && segs.len() >= 2 {
        let root = &segs[0];
        let op_seg = &segs[1];
        if is_pascal_case_first(&root.name) && is_mongoose_or_sequelize_op(&op_seg.name) {
            return Some(FlowEmission::DbQuery {
                entity_name: root.name.clone(),
                operation: classify_orm_op(&op_seg.name),
            });
        }
    }

    None
}

/// True when the file imports from a package that defines ORM entity-marker
/// decorators (`@Entity`, `@Table`, `@Schema`, `@Document`, `@Collection`).
/// Used to gate the entity-marker branch of `detect_decorator_flow_emission`
/// against type imports of the same names from unrelated packages.
pub(super) fn file_imports_orm_decorator_package(file_imports: &[ImportEntry]) -> bool {
    file_imports.iter().any(|imp| {
        let Some(m) = imp.module_path.as_deref() else { return false; };
        m == "typeorm"
            || m.starts_with("typeorm/")
            || m == "sequelize-typescript"
            || m == "@nestjs/mongoose"
            || m == "@mikro-orm/core"
            || m.starts_with("@mikro-orm/")
            || m == "@nestjs/typeorm"
    })
}

fn file_imports_mongoose_or_sequelize(file_imports: &[ImportEntry]) -> bool {
    file_imports.iter().any(|imp| {
        let Some(m) = imp.module_path.as_deref() else { return false; };
        m == "mongoose"
            || m.starts_with("mongoose/")
            || m == "sequelize"
            || m.starts_with("sequelize/")
            || m == "@sequelize/core"
            || m.starts_with("@sequelize/")
            || m == "sequelize-typescript"
    })
}

/// Classify an ORM method name into a `DbQueryOp`.
///
/// Wraps `DbQueryOp::from_method_name` (which folds case but matches only
/// against a fixed set of generic verbs) with ORM-specific compound forms
/// that the generic mapper would otherwise return `Other` for —
/// `findByIdAndUpdate`, `findByPk`, `createMany`, `bulkCreate`, and so on.
fn classify_orm_op(name: &str) -> DbQueryOp {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        // Compound Select forms not covered by from_method_name's literal set.
        "findbyid" | "findbypk" | "findunique" | "findfirst" | "findany"
        | "findfirstorthrow" | "finduniqueorthrow" | "findoneorfail"
        | "findoneby" | "findonebyorfail" | "findandcount" | "findandcountby"
        | "findby" | "findandcountall" | "estimateddocumentcount"
        | "countdocuments" | "countby" | "distinct" | "aggregate" | "groupby"
        | "has" | "exist" => DbQueryOp::Select,

        // Compound Insert forms.
        "createmany" | "createmanyandreturn" | "bulkcreate" | "insertmany"
        | "bulkbuild" => DbQueryOp::Insert,

        // Compound Update forms.
        "updatemany" | "updatemanyandreturn" | "updateone" | "replaceone"
        | "findbyidandupdate" | "findoneandupdate" | "findoneandreplace"
        | "softrestore" => DbQueryOp::Update,

        // Compound Delete forms.
        "deletemany" | "deleteone" | "destroyall" | "truncate"
        | "softdelete" | "softremove" | "findbyidanddelete"
        | "findbyidandremove" | "findoneanddelete" | "findoneandremove" => {
            DbQueryOp::Delete
        }

        // Compound Upsert forms.
        "createorupdate" | "saveorupdate" => DbQueryOp::Upsert,

        // Fall through to the shared classifier for the simple verbs it
        // already handles (find / findOne / findAll / create / save /
        // update / delete / remove / upsert / count / ...).
        _ => DbQueryOp::from_method_name(name),
    }
}

pub(super) fn is_pascal_case_first(s: &str) -> bool {
    s.chars().next().map_or(false, |c| c.is_ascii_uppercase())
}

fn is_camel_case_first(s: &str) -> bool {
    s.chars().next().map_or(false, |c| c.is_ascii_lowercase())
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Strip a `Repository` or `Repo` suffix (case-insensitive on the suffix
/// itself, preserving the prefix's casing) and PascalCase the remainder.
/// Returns `None` if no suffix is present or the prefix is empty.
fn repository_suffix_entity(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let prefix_len = if lower.ends_with("repository") {
        name.len() - "repository".len()
    } else if lower.ends_with("repo") {
        name.len() - "repo".len()
    } else {
        return None;
    };
    if prefix_len == 0 {
        return None;
    }
    Some(capitalize_first(&name[..prefix_len]))
}

/// Prisma-distinctive op set. `find` is intentionally excluded — it's a JS
/// Array method and would false-positive on three-segment chains rooted at
/// any object with a `find` property. Prisma's own catalogue uses the more
/// specific `findUnique` / `findFirst` / `findMany` forms.
fn is_prisma_op(name: &str) -> bool {
    matches!(
        name,
        "findUnique"
            | "findUniqueOrThrow"
            | "findFirst"
            | "findFirstOrThrow"
            | "findMany"
            | "create"
            | "createMany"
            | "createManyAndReturn"
            | "update"
            | "updateMany"
            | "updateManyAndReturn"
            | "upsert"
            | "delete"
            | "deleteMany"
            | "count"
            | "aggregate"
            | "groupBy"
    )
}

fn is_typeorm_op(name: &str) -> bool {
    matches!(
        name,
        "find"
            | "findOne"
            | "findOneBy"
            | "findOneOrFail"
            | "findOneByOrFail"
            | "findBy"
            | "findAndCount"
            | "findAndCountBy"
            | "save"
            | "insert"
            | "update"
            | "upsert"
            | "delete"
            | "remove"
            | "softDelete"
            | "softRemove"
            | "restore"
            | "count"
            | "countBy"
            | "exist"
            | "exists"
    )
}

fn is_mongoose_or_sequelize_op(name: &str) -> bool {
    matches!(
        name,
        // Mongoose statics
        "find"
            | "findOne"
            | "findById"
            | "findByIdAndUpdate"
            | "findByIdAndDelete"
            | "findByIdAndRemove"
            | "findOneAndUpdate"
            | "findOneAndDelete"
            | "findOneAndReplace"
            | "findOneAndRemove"
            | "create"
            | "insertMany"
            | "updateOne"
            | "updateMany"
            | "replaceOne"
            | "deleteOne"
            | "deleteMany"
            | "remove"
            | "countDocuments"
            | "estimatedDocumentCount"
            | "distinct"
            | "aggregate"
            | "exists"
            // Sequelize statics
            | "findAll"
            | "findByPk"
            | "findOrCreate"
            | "findOrBuild"
            | "findAndCountAll"
            | "bulkCreate"
            | "upsert"
            | "destroy"
            | "truncate"
            | "increment"
            | "decrement"
            // Shared / generic
            | "count"
            | "save"
    )
}
