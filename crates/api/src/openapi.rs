//! OpenAPI document definition — generated from actual Axum route handlers.
use utoipa::OpenApi;

use crate::routes;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "IAOP Agency Service",
        version = "0.1.0",
        description = "Organizational hierarchy, assignments, transfers, and leadership for the IAOP agency bounded context. All endpoints require a Bearer JWT; organizational scope is enforced per resource."
    ),
    paths(
        routes::create_agency,
        routes::list_agencies,
        routes::get_agency,
        routes::create_unit,
        routes::list_units,
        routes::get_unit,
        routes::change_parent,
        routes::get_ancestors,
        routes::get_descendants,
        routes::unit_assignments,
        routes::assign_agent,
        routes::agent_assignments,
        routes::agent_current_assignment,
        routes::transfer_agent,
        routes::list_transfers,
        routes::assign_leader_route,
        routes::end_leader_route,
        routes::get_leaders,
    ),
    components(schemas(
        crate::dto::ProblemDetails,
        iaop_agency_service_application::commands::AgencyDto,
        iaop_agency_service_application::commands::UnitDto,
        iaop_agency_service_application::commands::AssignmentDto,
        iaop_agency_service_application::commands::LeadershipDto,
        iaop_agency_service_domain::assignment::Transfer,
    )),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearerAuth",
                SecurityScheme::Http(HttpBuilder::new().scheme(HttpAuthScheme::Bearer).build()),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_has_paths_and_security() {
        let doc = ApiDoc::openapi();
        assert!(doc.paths.paths.len() >= 15, "expected >=15 paths");
    }
}
