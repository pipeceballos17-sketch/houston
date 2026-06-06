//! Map `CoreError` to HTTP status + `ErrorBody`.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use houston_engine_core::CoreError;
use houston_engine_protocol::{ErrorBody, ErrorCode, ErrorDetail};

pub struct ApiError(pub CoreError);

impl From<CoreError> for ApiError {
    fn from(e: CoreError) -> Self {
        Self(e)
    }
}

impl ApiError {
    /// 400 — the request was malformed or referenced something invalid.
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self(CoreError::BadRequest(msg.into()))
    }

    /// 404 — the addressed resource doesn't exist.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self(CoreError::NotFound(msg.into()))
    }

    /// 500 — an unexpected server-side failure.
    pub fn internal(msg: impl Into<String>) -> Self {
        Self(CoreError::Internal(msg.into()))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = self.0.code();
        let status = match code {
            ErrorCode::NotFound => StatusCode::NOT_FOUND,
            ErrorCode::Conflict => StatusCode::CONFLICT,
            ErrorCode::BadRequest => StatusCode::BAD_REQUEST,
            ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorCode::Forbidden => StatusCode::FORBIDDEN,
            ErrorCode::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::VersionMismatch => StatusCode::CONFLICT,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let details = self
            .0
            .kind()
            .map(|kind| serde_json::json!({ "kind": kind }));
        (
            status,
            Json(ErrorBody {
                error: ErrorDetail {
                    code,
                    message: self.0.to_string(),
                    details,
                },
            }),
        )
            .into_response()
    }
}
