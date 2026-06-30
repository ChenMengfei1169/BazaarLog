// Static asset serving: every frontend build artifact is embedded into the
// binary at compile time via rust-embed, so the shipped BazaarLog.exe is fully
// self-contained. Unknown paths fall back to index.html so client-side routing
// works on a fresh load.
use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, StatusCode};
use axum::response::Response;
use mime_guess::from_path;
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "static/"]
struct StaticAsset;

pub async fn static_handler(req: Request) -> Response {
    let path = trim_leading_slash(req.uri().path());
    // Track whether we are serving the SPA fallback so the correct MIME type
    // and cache headers are applied. Without this, a deep-link like
    // "/transactions" would be served as index.html but MIME-detected as
    // application/octet-stream because the path has no ".html" extension.
    let (asset, served_path) = match StaticAsset::get(path) {
        Some(asset) => (Some(asset), path),
        None => {
            // Treat any non-asset path as the SPA shell so deep links resolve
            // client-side. API routes are matched before this fallback.
            (StaticAsset::get("index.html"), "index.html")
        }
    };
    match asset {
        Some(file) => {
            let mime = from_path(served_path).first_or_octet_stream();
            let body = Body::from(file.data.into_owned());
            let mut resp = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, mime.as_ref())
                .body(body)
                .expect("static asset response");
            if served_path != "index.html" {
                // Hashed filenames make immutable assets safe to cache hard.
                resp.headers_mut().insert(
                    header::CACHE_CONTROL,
                    "public, max-age=31536000, immutable".parse().unwrap(),
                );
            } else {
                resp.headers_mut().insert(
                    header::CACHE_CONTROL,
                    "no-cache".parse().unwrap(),
                );
            }
            resp
        }
        None => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("not found"))
            .expect("not found response"),
    }
}

fn trim_leading_slash(p: &str) -> &str {
    p.strip_prefix('/').unwrap_or(p)
}
