use axum::http::StatusCode;

pub async fn handle() -> StatusCode {
    // TODO: actually revoke the token.
    //
    // Right now we just no-op this, it's not clear this actually needs to be
    // done and is annoying to implement.
    StatusCode::OK
}
