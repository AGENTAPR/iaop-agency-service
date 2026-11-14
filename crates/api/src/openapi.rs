//! OpenAPI document definition.
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "IAOP Agency Service",
        version = "0.1.0",
        description = "Organizational hierarchy, assignments, transfers"
    ),
    paths()
)]
pub struct ApiDoc;
