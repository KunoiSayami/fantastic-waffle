pub mod v1 {
    use super::auth::check_auth;
    use crate::file::FileEventHelper;
    use crate::server::DEFAULT_WAIT_TIME;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::{Extension, Json, Router};
    use http::{Request, Response};
    use hyper::Body;
    use publib::types::ExitExt;
    use serde_json::json;
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
                .fallback(|| async { (StatusCode::FORBIDDEN, "403 Forbidden") })
                .layer(AsyncRequireAuthorizationLayer::new(
                    |mut request: Request<Body>| async move {
                        if ["/file", "/query"]
                            .iter()
                            .any(|f| request.uri().path().starts_with(f))
                        {
                            if let Some(paths) = check_auth(&request, user_pool).await {
                                // Set `user_id` as a request extension so it can be accessed by other
                                // services down the stack.
                                request.extensions_mut().insert(paths);
                                Ok(request)
                            } else {
                                let unauthorized_response = Response::builder()
                                    .status(StatusCode::UNAUTHORIZED)
                                    .body(Body::empty())
                                    .unwrap();

                                Err(unauthorized_response)
                            }
                        } else {
                            Ok(request)
                        }
                    },
                ))
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
    ) -> impl IntoResponse {
        if let Some(receiver) = sender.send_request(paths).await {
            return if let Ok(result) =
                timeout(Duration::from_secs(DEFAULT_WAIT_TIME), receiver).await
            {
                match result {
                    Ok(result) => {
                        Ok(Json(json!({"status": 200, "result": result})).into_response())
                    }
                    Err(e) => Err(anyhow::Error::from(e).into()),
                }
            } else {
                Ok((StatusCode::GATEWAY_TIMEOUT).into_response())
            };
        }
        Ok((StatusCode::INTERNAL_SERVER_ERROR).into_response())
    }

    async fn get_file() -> impl IntoResponse {
        Ok((StatusCode::FORBIDDEN))
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

mod auth {
    use hyper::Request;
    use log::warn;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::RwLock;

    pub(super) async fn check_auth<B>(
        request: &Request<B>,
        pool: Arc<RwLock<HashMap<String, Vec<String>>>>,
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
pub use current::WebServer;
