use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use commy_sync::picture::{fits, picture_id};

use crate::Service;

pub async fn fetch(
    State(service): State<Arc<Service>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    let Some(id) = picture_id(&id) else {
        return (StatusCode::BAD_REQUEST, "not a picture id").into_response();
    };
    let key = service.limiter.key(service.address(peer, &headers));
    if service.limiter.open(key).is_err() {
        return (StatusCode::TOO_MANY_REQUESTS, "slow down").into_response();
    }
    match service.store.get_picture(&id, crate::now_ms()).await {
        Ok(Some(body)) => ([(header::CONTENT_TYPE, "application/octet-stream"), (header::CACHE_CONTROL, "no-store")], body).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no such picture").into_response(),
        Err(e) => {
            tracing::warn!("sync: could not read a picture: {e}");
            (StatusCode::SERVICE_UNAVAILABLE, "try again later").into_response()
        }
    }
}

pub async fn store(
    State(service): State<Arc<Service>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let Some(id) = picture_id(&id) else {
        return (StatusCode::BAD_REQUEST, "not a picture id").into_response();
    };
    if !fits(body.len()) {
        return (StatusCode::BAD_REQUEST, "not a sealed picture").into_response();
    }
    let key = service.limiter.key(service.address(peer, &headers));
    if service.limiter.append(key, &id, body.len()).is_err() {
        return (StatusCode::TOO_MANY_REQUESTS, "slow down").into_response();
    }
    match service.store.put_picture(&id, &body, crate::now_ms()).await {
        Ok(()) => {
            service.stats.stored_picture();
            StatusCode::NO_CONTENT.into_response()
        }
        Err(e) => {
            tracing::warn!("sync: could not keep a picture: {e}");
            (StatusCode::SERVICE_UNAVAILABLE, "try again later").into_response()
        }
    }
}
