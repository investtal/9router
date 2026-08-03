//! Admin user management (list + create).

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::auth::dashboard::{hash_password, AdminUser, DashboardUser, Role};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct UserPublic {
    pub id: String,
    pub username: String,
    pub role: Role,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    /// `"viewer"` | `"admin"` (default viewer).
    #[serde(default)]
    pub role: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/admin/users", get(list_users).post(create_user))
}

async fn list_users(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<UserPublic>>, AppError> {
    let rows = state
        .db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, username, role, created_at FROM users ORDER BY created_at ASC",
            )?;
            let rows = stmt
                .query_map([], |r| {
                    let role_s: String = r.get(2)?;
                    Ok(UserPublic {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        role: Role::parse(&role_s).unwrap_or(Role::Viewer),
                        created_at: r.get(3)?,
                    })
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .map_err(AppError::Internal)?;
    Ok(Json(rows))
}

async fn create_user(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<CreateUserRequest>,
) -> Result<Json<UserPublic>, AppError> {
    let username = body.username.trim();
    if username.is_empty() {
        return Err(AppError::BadRequest("username must not be empty".into()));
    }
    if body.password.is_empty() {
        return Err(AppError::BadRequest("password must not be empty".into()));
    }
    let role = match body.role.as_deref() {
        None | Some("") => Role::Viewer,
        Some(s) => Role::parse(s)
            .ok_or_else(|| AppError::BadRequest(format!("invalid role '{s}'")))?,
    };
    let password_hash = hash_password(&body.password)?;
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    let inserted = state
        .db
        .with_conn(|conn| {
            conn.execute(
                "INSERT INTO users (id, username, password_hash, oidc_sub, role, created_at)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
                params![id, username, password_hash, role.as_str(), created_at],
            )
        });
    match inserted {
        Ok(_) => Ok(Json(UserPublic {
            id,
            username: username.to_string(),
            role,
            created_at,
        })),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("UNIQUE") || msg.contains("unique") {
                Err(AppError::BadRequest(format!(
                    "username '{username}' already exists"
                )))
            } else {
                Err(AppError::Internal(e))
            }
        }
    }
}

/// Insert a user for tests / seeding (no HTTP).
pub fn insert_user(
    db: &crate::db::Db,
    username: &str,
    password: &str,
    role: Role,
) -> Result<DashboardUser, AppError> {
    let username = username.trim();
    if username.is_empty() || password.is_empty() {
        return Err(AppError::BadRequest("username/password required".into()));
    }
    let password_hash = hash_password(password)?;
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO users (id, username, password_hash, oidc_sub, role, created_at)
             VALUES (?1, ?2, ?3, NULL, ?4, ?5)",
            params![id, username, password_hash, role.as_str(), created_at],
        )
    })
    .map_err(AppError::Internal)?;
    Ok(DashboardUser {
        id,
        username: username.to_string(),
        role,
    })
}
