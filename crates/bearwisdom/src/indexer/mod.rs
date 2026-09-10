pub(crate) mod callback_lexical;
pub mod canonical_form;
pub mod changeset;
mod contract_bindings;
pub mod contract_filter;
pub mod demand;
pub(crate) mod demand_symbol_index;
pub mod embedded_regions;
pub mod ext_virtual_path;
pub mod external_parse_cache;
pub(crate) mod external_parse_payload;
pub(crate) mod external_parse_types;
pub mod flow;
mod flow_assignments;
pub(crate) mod flow_bindings;
pub mod flow_cfg;
#[cfg(test)]
#[path = "flow_config_tests.rs"]
mod flow_config_tests;
#[cfg(test)]
#[path = "flow_tests.rs"]
mod flow_tests;
pub mod full;
mod full_resolve_phase;
pub mod include_assembly;
pub mod incremental;
pub mod keywords;
pub mod lexical;
pub mod local_refs;
pub mod mem_probe;
pub mod module_resolution;
pub mod namespaces;
pub mod parse_file;
pub mod phase_timer;
pub mod plugin_state;
pub mod plugin_state_phase;
pub mod post_index;
pub mod query_builtins;
pub mod resolve_diff;
mod return_object_types;
pub mod secondary_scan;
pub mod service;
#[cfg(test)]
#[path = "service_tests.rs"]
mod service_tests;
pub mod stage_discover;
pub mod stage_link;
mod watch_filter;
pub use plugin_state::PluginStateBag;
pub mod programs;
pub mod project_context;
pub mod ref_cache;
pub mod resolve;
pub mod scip;
pub mod script_tag_deps;
pub mod symbol_ids;
pub mod test_file_detection;
pub mod write;
