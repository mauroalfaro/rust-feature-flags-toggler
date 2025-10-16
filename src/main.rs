use axum::{extract::{Path, State}, routing::{get, post, patch, delete}, Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::sync::RwLock;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    db: Pool<Sqlite>,
    cache: Arc<RwLock<HashMap<String, Flag>>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Flag {
    id: i64,
    key: String,
    enabled: bool,
    variants: Option<HashMap<String, u32>>,
    rollout: Option<u8>,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct CreateFlag {
    key: String,
    enabled: bool,
    variants: Option<HashMap<String, u32>>,
    rollout: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct UpdateFlag {
    enabled: Option<bool>,
    variants: Option<HashMap<String, u32>>,
    rollout: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct EvalRequest {
    key: String,
    user_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct EvalResponse {
    key: String,
    matched: bool,
    variant: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let env_filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info,tower_http=info".into());
    tracing_subscriber::registry().with(tracing_subscriber::EnvFilter::new(env_filter)).with(tracing_subscriber::fmt::layer()).init();

    let database_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite://flags.db".into());
    let pool = SqlitePoolOptions::new().max_connections(5).connect(&database_url).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS flags (\n            id INTEGER PRIMARY KEY AUTOINCREMENT,\n            key TEXT UNIQUE NOT NULL,\n            enabled INTEGER NOT NULL,\n            variants TEXT NULL,\n            rollout INTEGER NULL,\n            updated_at TEXT NOT NULL\n        )",
    )
    .execute(&pool)
    .await?;

    let state = AppState { db: pool, cache: Arc::new(RwLock::new(HashMap::new())) };

    let app = Router::new()
        .route("/health", get(health))
        .route("/flags", get(list_flags).post(create_flag))
        .route("/flags/:key", get(get_flag).patch(update_flag).delete(delete_flag))
        .route("/evaluate", post(evaluate))
        .with_state(state)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http());

    let addr: SocketAddr = std::env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8080".into()).parse()?;
    tracing::info!(%addr, "listening");
    axum::Server::bind(&addr).serve(app.into_make_service()).await?;
    Ok(())
}

async fn health() -> &'static str { "ok" }

async fn list_flags(State(state): State<AppState>) -> Result<Json<Vec<Flag>>, axum::http::StatusCode> {
    let rows = sqlx::query("SELECT id, key, enabled, variants, rollout, updated_at FROM flags")
        .fetch_all(&state.db)
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let out = rows.into_iter().map(row_to_flag).collect::<Result<Vec<_>, _>>()
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(out))
}

async fn get_flag(State(state): State<AppState>, Path(key): Path<String>) -> Result<Json<Flag>, axum::http::StatusCode> {
    let r = sqlx::query("SELECT id, key, enabled, variants, rollout, updated_at FROM flags WHERE key = ?")
        .bind(&key)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;
    let f = row_to_flag(r).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(f))
}

async fn create_flag(State(state): State<AppState>, Json(input): Json<CreateFlag>) -> Result<Json<Flag>, axum::http::StatusCode> {
    if input.rollout.is_some() && input.rollout.unwrap() > 100 { return Err(axum::http::StatusCode::BAD_REQUEST); }
    let variants_str = input.variants.as_ref().map(|v| serde_json::to_string(v).unwrap());
    sqlx::query("INSERT INTO flags (key, enabled, variants, rollout, updated_at) VALUES (?, ?, ?, ?, datetime('now'))")
        .bind(&input.key)
        .bind(if input.enabled { 1 } else { 0 })
        .bind(variants_str)
        .bind(input.rollout.map(|x| x as i64))
        .execute(&state.db)
        .await
        .map_err(|_| axum::http::StatusCode::CONFLICT)?;
    let r = sqlx::query("SELECT id, key, enabled, variants, rollout, updated_at FROM flags WHERE key = ?")
        .bind(&input.key)
        .fetch_one(&state.db)
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let f = row_to_flag(r).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(f))
}

async fn update_flag(State(state): State<AppState>, Path(key): Path<String>, Json(input): Json<UpdateFlag>) -> Result<Json<Flag>, axum::http::StatusCode> {
    if let Some(r) = input.rollout { if r > 100 { return Err(axum::http::StatusCode::BAD_REQUEST); } }
    let existing_row = sqlx::query("SELECT id, key, enabled, variants, rollout, updated_at FROM flags WHERE key = ?")
        .bind(&key)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;
    let existing = row_to_flag(existing_row).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let enabled = input.enabled.unwrap_or(existing.enabled);
    let variants = match (input.variants, existing.variants) { (Some(v), _) => Some(serde_json::to_string(&v).unwrap()), (None, v) => v.map(|vv| serde_json::to_string(&vv).unwrap()) };
    let rollout = input.rollout.map(|x| x as i64).or(existing.rollout.map(|x| x as i64));
    sqlx::query("UPDATE flags SET enabled = ?, variants = ?, rollout = ?, updated_at = datetime('now') WHERE key = ?")
        .bind(if enabled { 1 } else { 0 })
        .bind(variants)
        .bind(rollout)
        .bind(&existing.key)
        .execute(&state.db)
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let r = sqlx::query("SELECT id, key, enabled, variants, rollout, updated_at FROM flags WHERE key = ?")
        .bind(&existing.key)
        .fetch_one(&state.db)
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let f = row_to_flag(r).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(f))
}

async fn delete_flag(State(state): State<AppState>, Path(key): Path<String>) -> Result<(), axum::http::StatusCode> {
    let rows = sqlx::query("DELETE FROM flags WHERE key = ?")
        .bind(&key)
        .execute(&state.db)
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?
        .rows_affected();
    if rows == 0 { return Err(axum::http::StatusCode::NOT_FOUND); }
    Ok(())
}

async fn evaluate(State(state): State<AppState>, Json(req): Json<EvalRequest>) -> Result<Json<EvalResponse>, axum::http::StatusCode> {
    let r = sqlx::query("SELECT id, key, enabled, variants, rollout, updated_at FROM flags WHERE key = ?")
        .bind(&req.key)
        .fetch_optional(&state.db)
        .await
        .map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(axum::http::StatusCode::NOT_FOUND)?;
    let flag = row_to_flag(r).map_err(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR)?;
    let res = eval_flag(&flag, req.user_id.as_deref());
    Ok(Json(res))
}

fn row_to_flag(r: sqlx::sqlite::SqliteRow) -> Result<Flag, anyhow::Error> {
    let id = r.get::<i64,_>("id");
    let key = r.get::<String,_>("key");
    let enabled = r.get::<i64,_>("enabled") != 0;
    let variants_str = r.get::<Option<String>,_>("variants");
    let variants = match variants_str { Some(s) => Some(serde_json::from_str::<HashMap<String, u32>>(&s)?), None => None };
    let rollout = r.get::<Option<i64>,_>("rollout").map(|x| x as u8);
    let updated_at = r.get::<String,_>("updated_at");
    Ok(Flag { id, key, enabled, variants, rollout, updated_at })
}

fn eval_flag(flag: &Flag, user_id: Option<&str>) -> EvalResponse {
    let gate = match flag.rollout {
        None => true,
        Some(p) => match user_id { None => false, Some(uid) => { let mut hasher = blake3::Hasher::new(); hasher.update(flag.key.as_bytes()); hasher.update(b":"); hasher.update(uid.as_bytes()); let h = hasher.finalize(); (h.as_bytes()[0] % 100) < p } },
    };
    if !flag.enabled || !gate { return EvalResponse { key: flag.key.clone(), matched: false, variant: None }; }
    if let Some(vs) = &flag.variants {
        let total: u32 = vs.values().copied().sum();
        if total == 0 { return EvalResponse { key: flag.key.clone(), matched: true, variant: None }; }
        let pick = match user_id { None => 0, Some(uid) => { let mut hasher = blake3::Hasher::new(); hasher.update(flag.key.as_bytes()); hasher.update(b"/"); hasher.update(uid.as_bytes()); let hh = hasher.finalize(); let n = u32::from_le_bytes(hk(hh.as_bytes())); n % total } };
        let mut acc = 0u32;
        for (name, weight) in vs.iter() { acc += *weight; if pick < acc { return EvalResponse { key: flag.key.clone(), matched: true, variant: Some(name.clone()) }; } }
        return EvalResponse { key: flag.key.clone(), matched: true, variant: None };
    }
    EvalResponse { key: flag.key.clone(), matched: true, variant: None }
}

fn hk(b: &[u8]) -> [u8; 4] { [b[0], b[1], b[2], b[3]] }
