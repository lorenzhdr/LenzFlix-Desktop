//! Mock für die Konto-Seiten (W4-account): Scan einer einzelnen Bibliothek (`POST /Items/{id}/Refresh`).
//! Nur für Administratoren; das Ereignis steht als `Bibliothek gescannt:<id>` in [`crate::MockHandle::admin_events`].

use super::{S, Shared};
use axum::Router;
use axum::extract::Path;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use std::sync::Arc;

async fn refresh_item(axum::extract::State(shared): S, Path(id): Path<String>) -> Response {
    if !super::extra::is_admin(&shared) {
        return StatusCode::FORBIDDEN.into_response();
    }
    super::extra::push_event(&shared, format!("Bibliothek gescannt:{id}"));
    StatusCode::NO_CONTENT.into_response()
}

pub(crate) fn routes() -> Router<Arc<Shared>> {
    Router::new().route("/Items/{id}/Refresh", post(refresh_item))
}
