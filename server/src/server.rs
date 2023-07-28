pub mod v1 {
    use axum::http::StatusCode;
    use axum::{Json, Router};
    use serde_json::json;
    use tower::ServiceBuilder;
    use tower_http::auth::AddAuthorizationLayer;
    use tower_http::trace::TraceLayer;

    pub fn make_route(bind: String) {
        let router = Router::new()
            .route(
                "/",
                axum::routing::get(|| async {
                    Json(json!({"version": env!("CARGO_PKG_VERSION"), "status": 200}))
                }),
            )
            .fallback(|| async { (StatusCode::FORBIDDEN, "403 Forbidden") })
            .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()));
        let server_handler = axum_server::Handle::new();
        let server = tokio::spawn(
            axum_server::bind(bind.parse().unwrap())
                .handle(server_handler.clone())
                .serve(router.into_make_service()),
        );
    }

    //async fn get_file
}

mod auth {
    use crate::server::AUTH_POOL;
    use futures_core::future::BoxFuture;
    use http::{header::AUTHORIZATION, StatusCode};
    use hyper::{Body, Error, Request, Response};
    use log::warn;
    use tower::{service_fn, Service, ServiceBuilder, ServiceExt};
    use tower_http::auth::{AsyncAuthorizeRequest, AsyncRequireAuthorizationLayer};

    #[derive(Clone, Copy)]
    struct Auth;

    impl<B> AsyncAuthorizeRequest<B> for Auth
    where
        B: Send + Sync + 'static,
    {
        type RequestBody = B;
        type ResponseBody = Body;
        type Future = BoxFuture<'static, Result<Request<B>, Response<Self::ResponseBody>>>;

        fn authorize(&mut self, mut request: Request<B>) -> Self::Future {
            Box::pin(async {
                if let Some(user_id) = check_auth(&request).await {
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

    async fn check_auth<B>(request: &Request<B>) -> Option<Vec<String>> {
        let client_map = AUTH_POOL.get().unwrap().read().await;
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

    #[derive(Debug)]
    struct UserId(String);
}

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;
pub use v1 as current;
pub static AUTH_POOL: OnceLock<RwLock<HashMap<String, Vec<String>>>> = OnceLock::new();
