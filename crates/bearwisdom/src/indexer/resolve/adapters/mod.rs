// =============================================================================
// indexer/resolve/adapters — framework-specific FlowEmission adapters
//
// Each submodule recognises one framework surface (mailer template paths,
// Next.js route files, extractor-emitted routes/DbSets/plugin emissions) and
// converts it to one or more FlowEmissions the cross-file pairer consumes.
// =============================================================================

mod extracted;
mod mailer;
mod nextjs;

pub use extracted::append_db_route_consumer_emissions;
pub(crate) use extracted::{
    extracted_db_sets_to_emissions, extracted_routes_to_emissions,
    plugin_flow_emissions_to_emissions,
};
pub(crate) use mailer::mailer_template_name_for_path;
pub(crate) use nextjs::nextjs_route_consumer_emissions;
