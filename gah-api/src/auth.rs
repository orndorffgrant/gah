use dropshot::{ClientErrorStatusCode, HttpError, RequestContext};

use crate::ApiCtx;

fn unauthorized(msg: &str) -> HttpError {
    HttpError::for_client_error_with_status(
        Some(msg.to_string()),
        ClientErrorStatusCode::UNAUTHORIZED,
    )
}

/// Validate the request's bearer token against the token store.
pub async fn check(rqctx: &RequestContext<ApiCtx>) -> Result<(), HttpError> {
    let auth_header = rqctx
        .request
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| unauthorized("missing or invalid Authorization header"))?;

    if token.is_empty() {
        return Err(unauthorized("empty bearer token"));
    }

    let valid = rqctx
        .context()
        .token_store
        .validate(token)
        .await
        .map_err(|e| HttpError::for_internal_error(format!("token lookup failed: {e}")))?;

    if !valid {
        return Err(unauthorized("invalid bearer token"));
    }

    Ok(())
}

pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}
