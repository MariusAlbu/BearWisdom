// =============================================================================
// languages/typescript/flow_detectors — FlowEmission detection by kind
//
// The TS resolver emits cross-tier `FlowEmission` values when a ref site
// matches one of several well-known patterns. Detection is split by kind
// across four submodules:
//
//   * decorators — NestJS / Angular / TypeORM / gRPC decorator patterns
//   * chains     — HTTP / IPC / WebSocket / GraphQL / route-handler /
//                  config / feature-flag call chains
//   * db         — ORM call chains (Prisma / TypeORM / Mongoose / Sequelize)
//   * messaging  — async messaging (MQ / BgJob / RPC client / Mailer)
//
// Each submodule owns one detector kind plus its private helpers. Shared
// helpers (`first_arg_string`) live here; cross-kind constants
// (`canonical_rpc_key`, `BGJOB_QUEUE_BINDING_KEY`) live in the
// submodule that owns the primary user and are re-exported pub(super).
// =============================================================================

mod chains;
mod db;
mod decorators;
mod messaging;

use crate::types::CallArg;

// pub(crate) re-exports of the detector entry points the TypeScript resolver
// (and resolve_tests.rs) consume by name. Re-exporting keeps the public
// `super::flow_detectors::X` call path stable across the carve-up.
pub(crate) use chains::{
    detect_chain_flow_emission, detect_chain_route_consumer, detect_config_call_emission,
    detect_feature_flag_chain_emission, detect_member_access_config_emission,
    detect_member_access_feature_flag_emission, detect_trpc_chain_emission, parse_gql_operation,
};
pub(crate) use db::{
    detect_db_query_emission, detect_db_query_emission_with_imports,
};
pub(crate) use decorators::{
    detect_addservice_object_keys, detect_angular_injectable_emission,
    detect_decorator_flow_emission, detect_decorator_flow_emission_with_imports,
    detect_grpc_decorator_flow_emission, detect_route_decorator_flow_emission,
    join_route_segments, lookup_controller_prefix, CONTROLLER_PREFIX_KEY,
};
pub(crate) use messaging::{
    canonical_rpc_key, detect_bgjob_chain_emission, detect_mailer_chain_emission,
    detect_mq_chain_emission, detect_rpc_chain_emission, BGJOB_QUEUE_BINDING_KEY,
};

/// Extract the first argument as a plain string when it is a string literal
/// or a template literal. Returns an empty string when the argument is an
/// identifier or otherwise not statically determinable.
pub(super) fn first_arg_string(call_args: &[CallArg]) -> String {
    match call_args.first() {
        Some(CallArg::StringLit(s)) => s.clone(),
        Some(CallArg::TemplateLit(s)) => s.clone(),
        _ => String::new(),
    }
}