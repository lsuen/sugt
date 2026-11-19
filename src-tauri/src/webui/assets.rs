//! WebUI 静态资源层。
//!
//! CLI 是独立二进制（Linux tar.gz / 单 exe），运行时没有 dist 目录可用，
//! 所以前端产物必须在编译期嵌入。用 `include_dir` 把 `../webui-dist` 整个目录
//! 编进二进制；目录缺失时降级到内置占位页，保证 `cargo build --bin sugt-cli`
//! 在未跑 npm build 的 CI 上也能通过（占位页会提示如何生成真实前端）。

use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::Response;
use std::path::Path;

/// 真实前端产物（由 npm run build:web 生成到 src-tauri/webui-dist）。
#[allow(dead_code)]
static EMBEDDED: include_dir::Dir<'static> = include_dir::include_dir!("$CARGO_MANIFEST_DIR/webui-dist");

/// 占位首页：webui-dist 未生成时返回，提示构建方式而非白屏。
const FALLBACK_HTML: &str = r#"<!doctype html><html lang="zh"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1"><title>SUGT WebUI</title>
<style>body{font:15px/1.7 system-ui,sans-serif;max-width:620px;margin:12vh auto;padding:0 24px;color:#e8edf7;background:#070b14}
code{background:#1a2233;padding:2px 7px;border-radius:5px;color:#7fb4e8}h1{font-size:22px}</style></head>
<body><h1>SUGT WebUI</h1><p>前端资源未嵌入。请在项目根目录执行：</p>
<p><code>npm ci &amp;&amp; npm run build:web</code></p>
<p>然后重新构建 CLI：<code>cargo build --release --bin sugt-cli</code></p></body></html>"#;

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

/// 占位页 handler（正常嵌入后不会被调用，保留给文档与测试说明）。
#[allow(dead_code)]
pub fn fallback_response() -> Response {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .body(Body::from(FALLBACK_HTML))
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
