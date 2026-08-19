//! `clients` 模块核心逻辑测试（跨平台，与平台无关）。

use sugt_lib::clients;
use sugt_lib::model::AppConfig;

#[test]
fn claude_vars_only_use_auth_token() {
    let vars = clients::claude_vars("http://127.0.0.1:8787", "sugt-local-key");
    assert!(vars.contains_key("ANTHROPIC_AUTH_TOKEN"));
    assert!(!vars.contains_key("ANTHROPIC_API_KEY"));
    assert!(!vars.contains_key("CLAUDE_CODE_API_KEY"));
}

#[test]
fn codex_vars_only_use_openai_keys() {
    let vars = clients::codex_vars("http://127.0.0.1:8787", "sugt-local-key");
    assert!(vars.contains_key("OPENAI_BASE_URL"));
    assert!(vars.contains_key("OPENAI_API_KEY"));
    assert!(vars.contains_key("CODEX_API_KEY"));
    assert!(!vars.contains_key("ANTHROPIC_AUTH_TOKEN"));
}

#[test]
fn gateway_url_match_includes_v1_suffix() {
    let listen = "http://127.0.0.1:8787";
    assert!(clients::urls_point_to_gateway("http://127.0.0.1:8787/v1", listen));
    assert!(clients::urls_point_to_gateway("http://127.0.0.1:8787/", listen));
    assert!(!clients::urls_point_to_gateway("https://api.openai.com/v1", listen));
}

#[test]
fn print_launch_script_returns_string() {
    let text = clients::print_launch_script(&AppConfig::default());
    // 不强制格式，只验证不崩溃
    let _ = text;
}