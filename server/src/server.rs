pub mod v1 {
    use super::auth::check_auth;
    use crate::file::FileEventHelper;
    use crate::server::{WebResponse, DEFAULT_WAIT_TIME};
    use anyhow::anyhow;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::{Extension, Json, Router};
    use axum_macros::debug_handler;
    use http::{Request, Response};
    use hyper::Body;
    use publib::types::ExitExt;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;
    use tokio::sync::RwLock;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;
    use tower::ServiceBuilder;
    use tower_http::auth::AsyncRequireAuthorizationLayer;
    use tower_http::trace::TraceLayer;

    #[derive(Debug)]
    pub struct WebServer {
        join_handler: JoinHandle<std::io::Result<()>>,
        handler: axum_server::Handle,
    }

    impl WebServer {
        pub fn router_start(
            bind: String,
            user_pool: Arc<RwLock<HashMap<String, Vec<String>>>>,
            helper: FileEventHelper,
        ) -> Self {
            let router = Router::new()
                .route(
                    "/",
                    axum::routing::get(|| async {
                        Json(json!({"version": env!("CARGO_PKG_VERSION"), "status": 200}))
                    }),
                )
                .route("/file/*path", axum::routing::get(get_file))
                .route("/query", axum::routing::get(query))
                //.fallback(|| async { (StatusCode::FORBIDDEN, "403 Forbidden") })
                //.layer(AsyncRequireAuthorizationLayer::new(super::AuthLayer))
                .layer(Extension(helper))
                .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()));
            let server_handler = axum_server::Handle::new();
            let server = tokio::spawn(
                axum_server::bind(bind.parse().unwrap())
                    .handle(server_handler.clone())
                    .serve(router.into_make_service()),
            );
            Self::new(server, server_handler)
        }

        fn new(
            join_handler: JoinHandle<std::io::Result<()>>,
            handler: axum_server::Handle,
        ) -> Self {
            Self {
                join_handler,
                handler,
            }
        }
    }

    async fn query(
        Extension(sender): Extension<FileEventHelper>,
        Extension(paths): Extension<Vec<String>>,
    ) -> WebResponse {
        if let Some(receiver) = sender.send_request(paths).await {
            return if let Ok(result) =
                timeout(Duration::from_secs(DEFAULT_WAIT_TIME), receiver).await
            {
                match result {
                    Ok(result) => WebResponse::ok(Some(serde_json::to_value(result).unwrap())),
                    Err(e) => WebResponse::from(anyhow!("Query result error: {:?}", e)),
                }
            } else {
                WebResponse::gateway_timeout()
            };
        }
        WebResponse::forbidden(None)
    }

    async fn get_file() -> impl IntoResponse {
        (StatusCode::FORBIDDEN).into_response()
    }

    impl ExitExt for WebServer {
        fn _send_terminate(&self) -> Option<()> {
            Some(self.handler.shutdown())
        }

        fn is_finished(&self) -> bool {
            self.join_handler.is_finished()
        }
    }
}

mod types {
    use axum::response::{IntoResponse, Response};
    use axum::Json;
    use http::StatusCode;
    use serde_derive::Serialize;
    use serde_json::Value;

    #[derive(Clone, Debug, Serialize)]
    pub struct WebResponse {
        status: u16,
        result: Option<Value>,
        reason: Option<String>,
    }

    impl WebResponse {
        pub fn ok(result: Option<Value>) -> Self {
            Self::new(StatusCode::OK, result, None)
        }

        pub fn forbidden(reason: Option<String>) -> Self {
            Self::new(StatusCode::FORBIDDEN, None, reason)
        }

        pub fn internal_server_error(reason: Option<String>) -> Self {
            Self::new(StatusCode::INTERNAL_SERVER_ERROR, None, reason)
        }

        pub fn gateway_timeout() -> Self {
            Self::new(StatusCode::GATEWAY_TIMEOUT, None, None)
        }

        pub fn new(status: StatusCode, result: Option<Value>, reason: Option<String>) -> Self {
            Self {
                status: status.as_u16().into(),
                result,
                reason,
            }
        }
        pub fn result(&self) -> &Option<Value> {
            &self.result
        }
        pub fn reason(&self) -> &Option<String> {
            &self.reason
        }

        pub fn into_value(self) -> serde_json::Result<Value> {
            serde_json::to_value(self)
        }
    }

    impl IntoResponse for WebResponse {
        fn into_response(self) -> Response {
            (
                StatusCode::from_u16(self.status).unwrap(),
                Json(serde_json::to_string(&self).unwrap()),
            )
                .into_response()
        }
    }

    impl From<anyhow::Error> for WebResponse {
        fn from(value: anyhow::Error) -> Self {
            Self::internal_server_error(Some(value.to_string()))
        }
    }
}

mod auth {
    use axum::Extension;
    use axum_macros::debug_handler;
    use futures_core::future::BoxFuture;
    use log::warn;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    use http::{header::AUTHORIZATION, StatusCode};
    use hyper::{Body, Error, Request, Response};
    use tower::{service_fn, Service, ServiceBuilder, ServiceExt};
    use tower_http::auth::{AsyncAuthorizeRequest, AsyncRequireAuthorizationLayer};
    //use futures_util::future::BoxFuture;

    #[derive(Clone, Copy)]
    pub struct AuthLayer;

    impl<B> AsyncAuthorizeRequest<B> for AuthLayer
    where
        B: Send + Sync + 'static,
    {
        type RequestBody = B;
        type ResponseBody = Body;
        type Future = BoxFuture<'static, Result<Request<B>, Response<Self::ResponseBody>>>;

        fn authorize(&mut self, mut request: Request<B>) -> Self::Future {
            Box::pin(async {
                let pool = request
                    .extensions()
                    .get::<Arc<RwLock<HashMap<String, Vec<String>>>>>()
                    .unwrap();
                if let Some(user_id) = check_auth(&request, pool).await {
                    // Set `user_id` as a request extension so it can be accessed by other
                    // services down the stack.
                    request.extensions_mut().insert(user_id);

                    Ok(request)
                } else {
                    let unauthorized_response = Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Body::empty())
                        .unwrap();

                    Err(unauthorized_response)
                }
            })
        }
    }

    pub(super) async fn check_auth<B>(
        request: &Request<B>,
        pool: &Arc<RwLock<HashMap<String, Vec<String>>>>,
    ) -> Option<Vec<String>> {
        let client_map = pool.read().await;
        if let Some(bearer) = request.headers().get("Authorization") {
            let bearer = bearer
                .to_str()
                .inspect_err(|e| warn!("Unable decode authorization header: {:?}", e))
                .ok()?;
            if !bearer.starts_with("bearer ") {
                return None;
            }
            let (_, bearer) = bearer.split_once("bearer ").unwrap();
            let result = client_map.get(bearer)?;
            return Some(result.clone());
        }
        None
    }
}

use std::sync::OnceLock;
pub use v1 as current;

pub const DEFAULT_WAIT_TIME: u64 = 3;
pub static WAIT_TIME: OnceLock<u64> = OnceLock::new();
use auth::AuthLayer;
pub use current::WebServer;
pub use types::WebResponse;
