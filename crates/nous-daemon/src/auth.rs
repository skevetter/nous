use axum::extract::{Extension, Request};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct ApiKey(pub Arc<str>);

#[derive(Serialize)]
struct AuthError {
    error: AuthErrorBody,
}

#[derive(Serialize)]
struct AuthErrorBody {
    code: &'static str,
    message: &'static str,
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn unauthorized(message: &'static str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(AuthError {
            error: AuthErrorBody {
                code: "unauthorized",
                message,
            },
        }),
    )
        .into_response()
}

pub async fn require_api_key(
    Extension(key): Extension<ApiKey>,
    request: Request,
    next: Next,
) -> Response {
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(value) if value.starts_with("Bearer ") => {
            let token = &value[7..];
            if constant_time_eq(token, key.0.as_ref()) {
                next.run(request).await
            } else {
                unauthorized("invalid API key")
            }
        }
        Some(_) => unauthorized("authorization header must use Bearer scheme"),
        None => unauthorized("missing authorization header"),
    }
}
