pub mod v1 {
    use crate::configure::RwPoolType;
    use crate::file::FileEventHelper;
    use crate::server::auth::AuthLayer;
    use crate::server::{WebResponse, DEFAULT_WAIT_TIME};
    use anyhow::anyhow;
    use axum::body::StreamBody;
    use axum::extract::Path;
    use axum::response::IntoResponse;
    use axum::{Extension, Router};
    use http::header::InvalidHeaderValue;
    use http::{HeaderMap, HeaderValue, Request};
    use hyper::Body;
    use publib::{check_penetration, PATH_UTF8_ERROR};
    use serde_json::json;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;
    use tokio_util::io::ReaderStream;
    use tower::ServiceBuilder;
    use tower_http::auth::AsyncRequireAuthorizationLayer;
    use tower_http::trace::TraceLayer;

    pub fn router_start(
        bind: String,
        user_pool: Arc<RwPoolType>,
        helper: FileEventHelper,
    ) -> (JoinHandle<std::io::Result<()>>, axum_server::Handle) {
        let router = Router::new()
            .route(
                "/",
                axum::routing::get(|| async {
                    WebResponse::ok(Some(
                        json!({"version": env!("CARGO_PKG_VERSION"), "status": 200}),
                    ))
                }),
            )
            .route("/file/*path", axum::routing::get(get_file))
            .route("/query", axum::routing::get(query))
            .fallback(|| async { WebResponse::forbidden(None) })
            .route_layer(AsyncRequireAuthorizationLayer::new(AuthLayer))
            .layer(Extension(user_pool))
            .layer(Extension(helper))
            .layer(ServiceBuilder::new().layer(TraceLayer::new_for_http()));
        let server_handler = axum_server::Handle::new();
        let server = tokio::spawn(
            axum_server::bind(bind.parse().unwrap())
                .handle(server_handler.clone())
                .serve(router.into_make_service()),
        );
        (server, server_handler)
    }

    async fn query(
        Extension(sender): Extension<FileEventHelper>,
        request: Request<Body>,
    ) -> WebResponse {
        let paths = request.extensions().get::<Vec<String>>();

        if paths.is_none() {
            return WebResponse::internal_server_error_str(Some("Paths is None"));
        }

        if let Some(receiver) = sender.send_request(paths.unwrap().to_owned()).await {
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

    fn build_filename_value(filename: &str) -> Result<HeaderValue, InvalidHeaderValue> {
        HeaderValue::from_str(&format!("attachment; filename=\"{}\"", filename))
    }

    async fn get_file(
        Path(path): Path<String>,
        request: Request<Body>,
    ) -> Result<impl IntoResponse, WebResponse> {
        let paths = request.extensions().get::<Vec<String>>();
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::CONTENT_TYPE,
            "application/octet-stream".parse().unwrap(),
        );

        if paths.is_none() {
            return Err(WebResponse::internal_server_error_str(Some(
                "Paths is None",
            )));
        }

        // Check path penetration
        if !check_penetration(&path) {
            return Err(WebResponse::forbidden(None));
        }

        // Check request path is valid
        if !paths.unwrap().iter().any(|p| path.starts_with(p)) {
            return Err(WebResponse::forbidden(None));
        }

        let buf: &std::path::Path = path.as_ref();
        if buf.is_dir() {
            return Err(WebResponse::bad_request(Some("Request download directory")));
        }

        match buf.file_name() {
            None => Err(WebResponse::internal_server_error_str(Some(
                "Unable to get file name",
            ))),
            Some(filename) => {
                headers.insert(
                    http::header::CONTENT_DISPOSITION,
                    build_filename_value(filename.to_str().expect(PATH_UTF8_ERROR)).unwrap(),
                );
                match tokio::fs::File::open(path).await {
                    Ok(file) => {
                        let body = StreamBody::new(ReaderStream::new(file));

                        Ok((headers, body))
                    }
                    Err(e) => Err(WebResponse::from(anyhow!("Unable to read file: {:?}", e))),
                }
            }
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

        #[allow(unused)]
        pub fn forbidden_note(reason: &'static str) -> Self {
            Self::new(StatusCode::FORBIDDEN, None, Some(reason.to_string()))
        }

        pub fn internal_server_error(reason: Option<String>) -> Self {
            Self::new(StatusCode::INTERNAL_SERVER_ERROR, None, reason)
        }

        pub fn internal_server_error_str(reason: Option<&'static str>) -> Self {
            Self::internal_server_error(reason.map(|s| s.to_string()))
        }

        pub fn bad_request(reason: Option<&'static str>) -> Self {
            Self::new(StatusCode::BAD_REQUEST, None, reason.map(|s| s.to_string()))
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
        #[allow(unused)]
        pub fn result(&self) -> &Option<Value> {
            &self.result
        }
        #[allow(unused)]
        pub fn reason(&self) -> &Option<String> {
            &self.reason
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
    use axum::body::BoxBody;
    use axum::Extension;
    use log::warn;
    use std::sync::Arc;

    use crate::configure::RwPoolType;
    use futures_util::future::BoxFuture;
    use http::StatusCode;
    use hyper::{Request, Response};
    use tower_http::auth::AsyncAuthorizeRequest;

    #[derive(Clone, Copy)]
    pub struct AuthLayer;

    impl<B> AsyncAuthorizeRequest<B> for AuthLayer
    where
        B: Send + Sync + 'static,
    {
        type RequestBody = B;
        // https://github.com/tower-rs/tower-http/discussions/346
        type ResponseBody = BoxBody;
        type Future = BoxFuture<'static, Result<Request<B>, Response<Self::ResponseBody>>>;

        fn authorize(&mut self, mut request: Request<B>) -> Self::Future {
            Box::pin(async {
                let pool = request.extensions().get::<Arc<RwPoolType>>().unwrap();
                if let Some(user_id) = check_auth(&request, pool).await {
                    // Set `user_id` as a request extension so it can be accessed by other
                    // services down the stack.
                    request.extensions_mut().insert(Extension(user_id));

                    Ok(request)
                } else {
                    let unauthorized_response = Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(BoxBody::default())
                        .unwrap();

                    Err(unauthorized_response)
                }
            })
        }
    }

    pub(super) async fn check_auth<B>(
        request: &Request<B>,
        pool: &Arc<RwPoolType>,
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
pub const DEFAULT_WAIT_TIME_STR: &str = "3";
pub static WAIT_TIME: OnceLock<u64> = OnceLock::new();
pub use current::router_start;
pub use types::WebResponse;
