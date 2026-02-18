use crate::{
    AppState, Error, Result,
    db::{Database, NewUserParams, User, UserId},
    rpc::Principal,
};
use anyhow::Context as _;
use axum::{
    http::{self, Request, StatusCode},
    middleware::Next,
    response::IntoResponse,
};
use cloud_api_types::GetAuthenticatedUserResponse;
pub use rpc::auth::random_token;
use sha2::Digest;
use std::sync::Arc;

/// Validates the authorization header and adds an Extension<Principal> to the request.
/// Authorization: <user-id|github-login> <token>
///   <token> can be an access_token attached to that user, or an access token of an admin
///   or the string ADMIN_TOKEN:<config.api_token>.
/// Authorization: "dev-server-token" <token>
pub async fn validate_header<B>(mut req: Request<B>, next: Next<B>) -> impl IntoResponse {
    let mut auth_header = req
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .ok_or_else(|| {
            Error::http(
                StatusCode::UNAUTHORIZED,
                "missing authorization header".to_string(),
            )
        })?
        .split_whitespace();

    let state = req.extensions().get::<Arc<AppState>>().unwrap();

    let first = auth_header.next().unwrap_or("");
    if first == "dev-server-token" {
        Err(Error::http(
            StatusCode::UNAUTHORIZED,
            "Dev servers were removed in Zed 0.157 please upgrade to SSH remoting".to_string(),
        ))?;
    }

    let access_token = auth_header.next().ok_or_else(|| {
        Error::http(
            StatusCode::BAD_REQUEST,
            "missing access token in authorization header".to_string(),
        )
    })?;

    if let Ok(user_id) = first.parse::<i32>() {
        let user_id = UserId(user_id);
        let http_client = state.http_client.clone().expect("no HTTP client");

        let response = http_client
            .get(format!("{}/client/users/me", state.config.zed_cloud_url()))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("{user_id} {access_token}"))
            .send()
            .await
            .context("failed to validate access token")?;

        if let Ok(response) = response.error_for_status() {
            let response_body: GetAuthenticatedUserResponse = response
                .json()
                .await
                .context("failed to parse response body")?;

            let user_id = UserId(response_body.user.id);

            let user = state
                .db
                .get_user_by_id(user_id)
                .await?
                .with_context(|| format!("user {user_id} not found"))?;

            req.extensions_mut().insert(Principal::User(user));
            return Ok::<_, Error>(next.run(req).await);
        }

        return Err(Error::http(
            StatusCode::UNAUTHORIZED,
            "invalid credentials".to_string(),
        ));
    }

    let github_login = first.trim();
    if github_login.is_empty() {
        return Err(Error::http(
            StatusCode::BAD_REQUEST,
            "missing user id in authorization header".to_string(),
        ));
    }

    let github_login = github_login.to_ascii_lowercase();
    if !is_valid_github_login(&github_login) {
        return Err(Error::http(
            StatusCode::BAD_REQUEST,
            "invalid github login in authorization header".to_string(),
        ));
    }

    let Some(admin_token) = access_token.strip_prefix("ADMIN_TOKEN:") else {
        return Err(Error::http(
            StatusCode::UNAUTHORIZED,
            "invalid credentials".to_string(),
        ));
    };

    if state.config.api_token != admin_token {
        return Err(Error::http(
            StatusCode::UNAUTHORIZED,
            "invalid credentials".to_string(),
        ));
    }

    let user = get_or_create_user_for_trusted_login(&github_login, &state.db).await?;
    req.extensions_mut().insert(Principal::User(user));
    Ok::<_, Error>(next.run(req).await)
}

fn is_valid_github_login(github_login: &str) -> bool {
    if github_login.len() > 39 {
        return false;
    }

    let mut chars = github_login.chars();
    let Some(first_char) = chars.next() else {
        return false;
    };

    if !first_char.is_ascii_alphanumeric() {
        return false;
    }

    chars.all(|character| character.is_ascii_alphanumeric() || character == '-')
}

async fn get_or_create_user_for_trusted_login(
    github_login: &str,
    db: &Arc<Database>,
) -> Result<User> {
    if let Some(user) = db.get_user_by_github_login(github_login).await? {
        return Ok(user);
    }

    db.create_user(
        &format!("{github_login}@example.com"),
        None,
        false,
        NewUserParams {
            github_login: github_login.to_string(),
            github_user_id: synthetic_github_user_id(github_login),
        },
    )
    .await?;

    db.get_user_by_github_login(github_login)
        .await?
        .with_context(|| format!("user {github_login} not found after create"))
        .map_err(Into::into)
}

fn synthetic_github_user_id(github_login: &str) -> i32 {
    let digest = sha2::Sha256::digest(github_login);
    let mut bytes = [0_u8; 4];
    bytes.copy_from_slice(&digest[..4]);
    let id = i32::from_be_bytes(bytes) & 0x7fff_ffff;
    if id == 0 { 1 } else { id }
}
