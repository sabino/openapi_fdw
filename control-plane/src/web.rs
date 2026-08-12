use crate::auth::{AuthState, has_mutation_header};
use crate::db;
use crate::model::{
    ApplyRequest, Bundle, DeleteRequest, MutationResult, SampleQuery, SourceDefinition,
};
use crate::ui;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Form, Path, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use deadpool_postgres::Pool;
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use tracing::error;

const APP_CSS: &str = include_str!("../assets/app.css");
const APP_JS: &str = include_str!("../assets/app.js");

#[derive(Clone)]
pub struct AppState {
    pub pool: Pool,
    pub auth: Arc<AuthState>,
}

pub fn router(state: AppState) -> Router {
    let protected = Router::new()
        .route("/", get(dashboard))
        .route("/logout", post(logout))
        .route("/api/v1/state", get(control_state))
        .route("/api/v1/discover", post(discover))
        .route("/api/v1/sources/plan", post(plan_source))
        .route("/api/v1/sources", post(apply_source))
        .route("/api/v1/sources/{name}", delete(delete_source))
        .route(
            "/api/v1/sources/{source}/tables/{schema}/{table}/rows",
            get(sample_rows),
        )
        .route("/api/v1/export", get(export_bundle))
        .route("/api/v1/import/plan", post(plan_import))
        .route("/api/v1/import/apply", post(apply_import))
        .layer(DefaultBodyLimit::max(8 * 1024 * 1024))
        .route_layer(middleware::from_fn_with_state(
            state.auth.clone(),
            require_authentication,
        ));

    Router::new()
        .route("/healthz", get(health))
        .route("/login", get(login_page).post(login))
        .route("/assets/app.css", get(stylesheet))
        .route("/assets/app.js", get(javascript))
        .merge(protected)
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}

async fn health(State(state): State<AppState>) -> Response {
    match db::health(&state.pool).await {
        Ok(()) => (StatusCode::OK, Json(json!({ "status": "ok" }))).into_response(),
        Err(error) => {
            error!(%error, "health check failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "status": "unavailable" })),
            )
                .into_response()
        }
    }
}

async fn login_page() -> Html<String> {
    Html(ui::login(None))
}

#[derive(Deserialize)]
struct LoginForm {
    token: String,
}

async fn login(State(state): State<AppState>, Form(form): Form<LoginForm>) -> Response {
    if !state.auth.verify_token(&form.token) {
        return (
            StatusCode::UNAUTHORIZED,
            Html(ui::login(Some("That token was not accepted."))),
        )
            .into_response();
    }
    let mut response = Redirect::to("/").into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, state.auth.session_cookie());
    response
}

async fn logout(State(state): State<AppState>) -> Response {
    let mut response = Redirect::to("/login").into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, state.auth.clear_cookie());
    response
}

async fn dashboard() -> Html<String> {
    Html(ui::dashboard())
}

async fn stylesheet() -> Response {
    static_asset(APP_CSS, "text/css; charset=utf-8")
}

async fn javascript() -> Response {
    static_asset(APP_JS, "text/javascript; charset=utf-8")
}

async fn control_state(
    State(state): State<AppState>,
) -> Result<Json<crate::model::ControlState>, ApiError> {
    db::state(&state.pool)
        .await
        .map(Json)
        .map_err(ApiError::internal)
}

async fn discover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(source): Json<SourceDefinition>,
) -> Result<Json<crate::model::Discovery>, ApiError> {
    require_mutation(&headers)?;
    db::discover(&state.pool, &source)
        .await
        .map(Json)
        .map_err(ApiError::unprocessable)
}

async fn plan_source(
    headers: HeaderMap,
    Json(request): Json<ApplyRequest>,
) -> Result<Json<MutationResult>, ApiError> {
    require_mutation(&headers)?;
    let bundle = Bundle::new(vec![request.source]);
    let sql = db::plan_bundle(&bundle, request.replace)
        .await
        .map_err(ApiError::unprocessable)?;
    Ok(Json(MutationResult {
        ok: true,
        message: "Review this redacted SQL before applying it.".to_string(),
        sql,
    }))
}

async fn apply_source(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ApplyRequest>,
) -> Result<Json<MutationResult>, ApiError> {
    require_mutation(&headers)?;
    db::apply_source(&state.pool, request.source, request.replace)
        .await
        .map(Json)
        .map_err(ApiError::unprocessable)
}

async fn delete_source(
    State(state): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    Json(request): Json<DeleteRequest>,
) -> Result<Json<MutationResult>, ApiError> {
    require_mutation(&headers)?;
    db::delete_source(&state.pool, &name, &request.confirm)
        .await
        .map(Json)
        .map_err(ApiError::unprocessable)
}

async fn sample_rows(
    State(state): State<AppState>,
    Path((source, schema, table)): Path<(String, String, String)>,
    Query(query): Query<SampleQuery>,
) -> Result<Json<crate::model::SampleResult>, ApiError> {
    db::sample_rows(&state.pool, &source, &schema, &table, &query)
        .await
        .map(Json)
        .map_err(ApiError::unprocessable)
}

async fn export_bundle(State(state): State<AppState>) -> Result<Response, ApiError> {
    let bundle = db::export_bundle(&state.pool)
        .await
        .map_err(ApiError::internal)?;
    let body = serde_json::to_vec_pretty(&bundle).map_err(ApiError::internal)?;
    let mut response = Response::new(Body::from(body));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=openapi-fdw-setup.json"),
    );
    Ok(response)
}

#[derive(Default, Deserialize)]
struct ReplaceQuery {
    #[serde(default)]
    replace: bool,
}

async fn plan_import(
    headers: HeaderMap,
    Query(query): Query<ReplaceQuery>,
    Json(bundle): Json<Bundle>,
) -> Result<Json<MutationResult>, ApiError> {
    require_mutation(&headers)?;
    let sql = db::plan_bundle(&bundle, query.replace)
        .await
        .map_err(ApiError::unprocessable)?;
    Ok(Json(MutationResult {
        ok: true,
        message: format!("{} source(s) validated", bundle.sources.len()),
        sql,
    }))
}

async fn apply_import(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ReplaceQuery>,
    Json(bundle): Json<Bundle>,
) -> Result<Json<MutationResult>, ApiError> {
    require_mutation(&headers)?;
    let count = bundle.sources.len();
    let sql = db::apply_bundle(&state.pool, &bundle, query.replace)
        .await
        .map_err(ApiError::unprocessable)?;
    Ok(Json(MutationResult {
        ok: true,
        message: format!("{count} source(s) imported"),
        sql,
    }))
}

async fn require_authentication(
    State(auth): State<Arc<AuthState>>,
    request: Request,
    next: Next,
) -> Response {
    if auth.authorized(request.headers()) {
        return next.run(request).await;
    }
    if request.uri().path().starts_with("/api/") {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "authentication required" })),
        )
            .into_response();
    }
    Redirect::to("/login").into_response()
}

async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn static_asset(content: &'static str, content_type: &'static str) -> Response {
    let mut response = Response::new(Body::from(content));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=3600"),
    );
    response
}

fn require_mutation(headers: &HeaderMap) -> Result<(), ApiError> {
    if has_mutation_header(headers) {
        Ok(())
    } else {
        Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "missing control-plane mutation header",
        ))
    }
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn unprocessable(error: anyhow::Error) -> Self {
        Self::new(StatusCode::UNPROCESSABLE_ENTITY, format!("{error:#}"))
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        error!(%error, "control-plane request failed");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal control-plane error",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}
