use axum::Router;

mod auth;
mod cas;

pub fn routes() -> Router {
    Router::new()
        .nest("/auth", auth::routes())
        .nest("/cas", cas::routes())
}
