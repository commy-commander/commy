use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};

const ANY_SITE: HeaderValue = HeaderValue::from_static("*");
const ASKED_METHODS: HeaderValue = HeaderValue::from_static("GET, POST");
const ASKED_HEADERS: HeaderValue = HeaderValue::from_static("content-type");
const REMEMBER_SECS: HeaderValue = HeaderValue::from_static("86400");

pub async fn open_to_any_site(mut response: Response) -> Response {
    response.headers_mut().insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, ANY_SITE);
    response
}

pub async fn preflight() -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, ASKED_METHODS);
    headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, ASKED_HEADERS);
    headers.insert(header::ACCESS_CONTROL_MAX_AGE, REMEMBER_SECS);
    response
}
