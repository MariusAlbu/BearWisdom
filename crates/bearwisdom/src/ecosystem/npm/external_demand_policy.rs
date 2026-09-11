use std::collections::HashSet;

use crate::ecosystem::externals::ExternalDepRoot;
use crate::indexer::demand::DemandSet;

pub(super) fn ambient_global_packages(roots: &[ExternalDepRoot]) -> HashSet<String> {
    crate::ecosystem::npm::ambient_global_packages(roots)
}

pub(super) fn external_demand<'a>(
    relative_path: &str,
    demand: &'a DemandSet,
    ambient_global_packages: &HashSet<String>,
) -> crate::ecosystem::external_policy::ExternalDemandDecision<'a> {
    match crate::ecosystem::npm::external_demand_policy(
        relative_path,
        demand,
        ambient_global_packages,
    ) {
        crate::ecosystem::npm::ExternalDemandPolicy::Demand(symbols) => {
            crate::ecosystem::external_policy::ExternalDemandDecision::Filter(symbols)
        }
        crate::ecosystem::npm::ExternalDemandPolicy::Full => {
            crate::ecosystem::external_policy::ExternalDemandDecision::Full
        }
        crate::ecosystem::npm::ExternalDemandPolicy::Unmatched => {
            crate::ecosystem::external_policy::ExternalDemandDecision::Decline
        }
    }
}
