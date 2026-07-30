use reqwest::StatusCode;

pub fn format_bind_error(addr: std::net::SocketAddr, err: &std::io::Error) -> String {
    let text = err.to_string().to_lowercase();
    if text.contains("address already in use")
        || text.contains("10048")
        || text.contains("addrinuse")
        || text.contains("通常每个套接字地址")
    {
        return format!("端口 {} 已被占用：请停止占用该端口的程序，或在配置中更换监听端口", addr.port());
    }
    if text.contains("10013")
        || text.contains("access denied")
        || text.contains("拒绝访问")
        || text.contains("permission denied")
    {
        return format!(
            "端口 {} 无法监听（系统拒绝访问）：Windows 可能保留了该端口段，请在配置中将 port 改为 9878 等未占用端口，并点击「修复接管」",
            addr.port()
        );
    }
    format!("无法监听 {}：{}", addr, err)
}

pub fn format_gateway_start_error(err: &anyhow::Error) -> String {
    let text = err.to_string().to_lowercase();
    if text.contains("监听地址无效") {
        return format!("监听地址无效，请检查 host/port 配置：{}", err);
    }
    format!("启动网关失败：{}", err)
}

pub fn classify_request_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        return "连接超时：请检查网络、代理或 Base URL 是否可达".to_string();
    }
    if err.is_connect() {
        return "无法连接上游服务器：请检查网络、防火墙、代理与 Base URL".to_string();
    }
    if err.is_body() || err.is_decode() {
        return format!("响应解析失败：{}", err);
    }
    format!("网络请求失败：{}", err)
}

pub fn classify_http_error(status: StatusCode, body: &str) -> String {
    let code = status.as_u16();
    let body_lower = body.to_lowercase();

    match code {
        401 => "鉴权失败 (401)：API Key 无效或未授权".to_string(),
        403 => "访问被拒绝 (403)：API Key 无权限或账号受限".to_string(),
        404 => {
            if body_lower.contains("model") {
                "模型不存在 (404)：请检查模型名称是否与上游一致".to_string()
            } else {
                "接口不存在 (404)：请检查 Base URL 与协议类型（OpenAI 需 /v1，Anthropic 通常不带 /v1）"
                    .to_string()
            }
        }
        400 => {
            if body_lower.contains("model") {
                "请求参数错误 (400)：模型名可能不正确或不被上游支持".to_string()
            } else {
                format!("请求参数错误 (400)：{}", truncate_body(body))
            }
        }
        429 => "请求过于频繁 (429)：已被上游限流，请稍后重试".to_string(),
        500..=599 => format!("上游服务异常 (HTTP {})：请稍后重试或切换备用模型", code),
        _ => format!("上游返回 HTTP {}：{}", code, truncate_body(body)),
    }
}

pub fn format_connection_test_error(status: Option<StatusCode>, body: &str, err: Option<&reqwest::Error>) -> String {
    if let Some(err) = err {
        return classify_request_error(err);
    }
    if let Some(status) = status {
        return classify_http_error(status, body);
    }
    "连接测试失败：未知错误".to_string()
}

fn truncate_body(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return "无详细错误信息".to_string();
    }
    let one_line = trimmed.replace('\n', " ");
    if one_line.chars().count() > 120 {
        format!("{}…", one_line.chars().take(120).collect::<String>())
    } else {
        one_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_auth_error() {
        let msg = classify_http_error(StatusCode::UNAUTHORIZED, r#"{"error":"invalid api key"}"#);
        assert!(msg.contains("401"));
    }

    #[test]
    fn classifies_model_not_found() {
        let msg = classify_http_error(StatusCode::NOT_FOUND, r#"{"error":{"message":"model not found"}}"#);
        assert!(msg.contains("模型"));
    }
}
