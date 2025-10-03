use axum::http::StatusCode;

pub async fn handle() -> StatusCode {
    // TODO: actually revoke the token.
    // Right now we just no-op this.
    StatusCode::OK
}
