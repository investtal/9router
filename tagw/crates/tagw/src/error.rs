use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("upstream: {0}")]
    Upstream(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::Upstream(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            // Never surface internal details (tokens, paths, SQL) to HTTP clients.
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into()),
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn response_json(err: AppError) -> (StatusCode, serde_json::Value) {
        let res = err.into_response();
        let status = res.status();
        let bytes = to_bytes(res.into_body(), 64 * 1024).await.expect("body");
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        (status, v)
    }

    #[tokio::test]
    async fn internal_error_response_does_not_leak_token() {
        let secret = "sk-super-secret-token-do-not-leak";
        let err = AppError::Internal(anyhow::anyhow!(
            "oauth refresh failed with bearer {secret} and refresh_token=rt-also-secret"
        ));
        // Display still has the detail for server logs — HTTP body must not.
        assert!(err.to_string().contains(secret));

        let (status, body) = response_json(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let msg = body["error"].as_str().expect("error string");
        assert_eq!(msg, "internal error");
        assert!(
            !msg.contains(secret) && !body.to_string().contains(secret),
            "response must not contain secret: {body}"
        );
        assert!(!body.to_string().contains("rt-also-secret"));
        assert!(!body.to_string().contains("refresh_token"));
    }

    #[tokio::test]
    async fn bad_request_still_surfaces_safe_message() {
        let (status, body) = response_json(AppError::BadRequest("name must not be empty".into())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"]
            .as_str()
            .unwrap_or("")
            .contains("name must not be empty"));
    }

    #[tokio::test]
    async fn unauthorized_is_generic() {
        let (status, body) = response_json(AppError::Unauthorized).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "unauthorized");
    }
}
