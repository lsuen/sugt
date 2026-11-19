//! WebUI 静态资源层。
//!
//! CLI 是独立二进制（Linux tar.gz / 单 exe），运行时没有 dist 目录可用，
//! 所以前端产物必须在编译期嵌入。用 `include_dir` 把 `../webui-dist` 整个目录
//! 编进二进制；`webui-dist` 必须存在（仓库里保留占位 `index.html`），
//! 真实页面由 `npm run build:web` 覆盖生成。

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::Response;
use std::path::Path;

/// 真实前端产物（由 npm run build:web 生成到 src-tauri/webui-dist）。
static EMBEDDED: include_dir::Dir<'static> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/webui-dist");

fn content_type(path: &str) -> &'static str {
    match Path::new(path).extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") | Some("mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("ico") => "image/x-icon",
        Some("woff") | Some("woff2") => "font/woff2",
        Some("map") => "application/json",
        _ => "application/octet-stream",
    }
}

fn embedded_file(rel: &str) -> Option<(&'static [u8], &'static str)> {
    let rel = rel.trim_start_matches('/');
    let target = if rel.is_empty() { "index.html" } else { rel };
    let file = EMBEDDED.get_file(target)?;
    Some((file.contents(), content_type(target)))
}

/// SPA 静态资源 handler：精确命中文件则直出，否则回退 index.html 交给前端路由。
pub async fn serve_static(uri: Uri) -> Response {
    let path = uri.path();
    if let Some((bytes, ctype)) = embedded_file(path) {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, ctype)
            .body(Body::from(bytes))
            .unwrap();
    }
    // SPA fallback：无扩展名的路径回退首页；带扩展名说明是真缺资源，返回 404。
    let has_ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some();
    if !has_ext {
        if let Some((bytes, ctype)) = embedded_file("index.html") {
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, ctype)
                .body(Body::from(bytes))
                .unwrap();
        }
    }
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from("404 Not Found"))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_mapping() {
        assert_eq!(content_type("a.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("a.wasm"), "application/octet-stream");
        assert_eq!(content_type("a.webp"), "image/webp");
    }
}
