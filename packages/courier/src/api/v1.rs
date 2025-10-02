use axum::Router;

pub mod auth;
pub mod cas;

pub fn router() -> Router {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/cas", cas::router())
}
