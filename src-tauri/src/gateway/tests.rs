//! `gateway` 模块单元测试（由 gateway.rs 外置，行为不变）。
//! 作者：sun / 孙文龙

use super::*;
use crate::anthropic_adapter::{anthropic_to_openai, openai_json_to_anthropic_json};

#[test]
fn rewrites_json_model() {
    let body = Bytes::from_static(br#"{"model":"old","messages":[]}"#);
    let rewritten = rewrite_model(
        body,
        "new-model",
        Some(crate::openai_responses_adapter::DEFAULT_CODEX_MAX_TOKENS),
    );
    let value: Value = serde_json::from_slice(&rewritten).unwrap();
    assert_eq!(value["model"], "new-model");
}

#[test]
fn rewrite_model_respects_default_max_tokens_switch() {
    let body = Bytes::from_static(br#"{"model":"old","messages":[]}"#);

    // 规则关闭（None）：不填 max_tokens。
    let suppressed = rewrite_model(body.clone(), "m", None);
    let value: Value = serde_json::from_slice(&suppressed).unwrap();
    assert!(value.get("max_tokens").is_none(), "关闭后不得自动填 max_tokens");

    // 规则覆盖：用规则值。
    let overridden = rewrite_model(body.clone(), "m", Some(4096));
    let value: Value = serde_json::from_slice(&overridden).unwrap();
    assert_eq!(value["max_tokens"], 4096);

    // 请求自带 max_tokens 时保持原值。
    let explicit = Bytes::from_static(br#"{"model":"old","max_tokens":128}"#);
    let kept = rewrite_model(explicit, "m", Some(4096));
    let value: Value = serde_json::from_slice(&kept).unwrap();
    assert_eq!(value["max_tokens"], 128);
}

#[test]
fn rule_body_fields_parse_json_values_and_fallback_to_string() {
    let effects = RuleEffects {
        body_fields: vec![
            ("temperature".to_string(), "0.2".to_string()),
            ("top_k".to_string(), "40".to_string()),
            ("stream".to_string(), "true".to_string()),
            ("metadata".to_string(), r#"{"a":1}"#.to_string()),
            ("user".to_string(), "alice".to_string()),
        ],
        ..Default::default()
    };
    let body = Bytes::from_static(br#"{"model":"m"}"#);
    let out = apply_rule_body_fields(body, &effects);
    let value: Value = serde_json::from_slice(&out).unwrap();

    assert_eq!(value["temperature"], 0.2, "数字按 JSON 解析");
    assert_eq!(value["top_k"], 40, "整数按 JSON 解析");
    assert_eq!(value["stream"], true, "布尔按 JSON 解析");
    assert_eq!(value["metadata"], serde_json::json!({"a": 1}), "对象按 JSON 解析");
    assert_eq!(value["user"], "alice", "非 JSON 回退为字符串");
}

#[test]
fn rule_body_fields_skip_non_object_body() {
    let effects = RuleEffects {
        body_fields: vec![("a".to_string(), "1".to_string())],
        ..Default::default()
    };
    for raw in [&b"[1,2]"[..], &b"not json"[..]] {
        let body = Bytes::copy_from_slice(raw);
        assert_eq!(
            apply_rule_body_fields(body.clone(), &effects),
            body,
            "非对象请求体必须原样返回"
        );
    }
}

#[test]
fn rule_headers_never_override_auth_or_protocol_headers() {
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::AUTHORIZATION,
        HeaderValue::from_static("Bearer client-key"),
    );
    headers.insert("x-keep", HeaderValue::from_static("client"));
    let effects = RuleEffects {
        headers: vec![
            ("Authorization".to_string(), "Bearer evil".to_string()),
            ("x-api-key".to_string(), "evil".to_string()),
            ("anthropic-version".to_string(), "1999-01-01".to_string()),
            ("x-keep".to_string(), "rule".to_string()),
            ("X-Added".to_string(), "1".to_string()),
        ],
        ..Default::default()
    };
    let request = copy_forward_headers(
        client.post("http://127.0.0.1:1/"),
        &headers,
        "real-key",
        AuthMode::OpenAi,
        &effects.headers,
    )
    .unwrap()
    .build()
    .unwrap();

    assert_eq!(
        request.headers().get(http::header::AUTHORIZATION).unwrap(),
        "Bearer real-key",
        "规则不得覆盖网关自己的鉴权头"
    );
    assert_eq!(
        request.headers().get_all(http::header::AUTHORIZATION).iter().count(),
        1,
        "同名头必须覆盖而不是追加"
    );
    assert!(request.headers().get("x-api-key").is_none());
    assert!(request.headers().get("anthropic-version").is_none());
    assert_eq!(request.headers().get("x-keep").unwrap(), "rule", "普通头以规则为准");
    assert_eq!(request.headers().get("x-added").unwrap(), "1");
}

#[test]
fn default_max_tokens_follows_rule_suppression() {
    let mut effects = RuleEffects {
        max_tokens: Some(4096),
        ..Default::default()
    };
    assert_eq!(rule_default_max_tokens(&effects, true), Some(4096));

    effects.max_tokens = None;
    effects.max_tokens_suppressed = true;
    assert_eq!(rule_default_max_tokens(&effects, true), None, "codex 语境可抑制");
    assert_eq!(
        rule_default_max_tokens(&effects, false),
        Some(crate::openai_responses_adapter::DEFAULT_CODEX_MAX_TOKENS),
        "Claude 路径保持原默认"
    );
}

/// 网关层：规则按 /v1/responses 语境命中，并在出站 header / body 上落地。
#[tokio::test]
async fn gateway_applies_request_rules_to_outbound_request() {
    use crate::request_rules::{RequestRule, RuleAction, RuleScope};

    let state = GatewayState::new(AppConfig::default(), None).expect("网关状态可构造");
    let rules = vec![
        RequestRule {
            id: "codex-extra".to_string(),
            name: "codex 加头".to_string(),
            scope: RuleScope::Codex,
            enabled: true,
            actions: vec![
                RuleAction::SetHeader {
                    key: "X-Rule-Mark".to_string(),
                    value: "1".to_string(),
                },
                RuleAction::SetBodyField {
                    key: "temperature".to_string(),
                    value: "0.3".to_string(),
                },
            ],
        },
        RequestRule {
            id: "agent-extra".to_string(),
            name: "其他 agent 加头".to_string(),
            scope: RuleScope::Agent {
                label: "trae".to_string(),
            },
            enabled: true,
            actions: vec![RuleAction::SetHeader {
                key: "X-Agent-Mark".to_string(),
                value: "1".to_string(),
            }],
        },
    ];
    state.set_request_rules(rules).await;

    let mut headers = HeaderMap::new();
    headers.insert(
        http::header::USER_AGENT,
        HeaderValue::from_static("codex_cli_rs/0.20.0"),
    );
    let (effects, is_codex) = request_rule_effects(&state, &headers, true).await;
    assert!(is_codex);
    assert_eq!(
        effects.headers,
        vec![("X-Rule-Mark".to_string(), "1".to_string())],
        "只应命中 codex 作用域"
    );

    let body = apply_rule_body_fields(Bytes::from_static(br#"{"model":"m"}"#), &effects);
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["temperature"], 0.3);
}

#[test]
fn maps_volcengine_coding_v3_responses() {
    let target = build_target_url(
        "https://ark.cn-beijing.volces.com/api/coding/v3",
        "v1/responses",
        "",
    );
    assert_eq!(
        target,
        "https://ark.cn-beijing.volces.com/api/coding/v3/responses"
    );
}

#[test]
fn maps_volcengine_coding_v3_chat_completions() {
    let target = build_target_url(
        "https://ark.cn-beijing.volces.com/api/coding/v3",
        "v1/chat/completions",
        "",
    );
    assert_eq!(
        target,
        "https://ark.cn-beijing.volces.com/api/coding/v3/chat/completions"
    );
}

#[test]
fn maps_volcengine_v3_chat_completions() {
    let target = build_target_url(
        "https://ark.cn-beijing.volces.com/api/v3",
        "v1/chat/completions",
        "",
    );
    assert_eq!(
        target,
        "https://ark.cn-beijing.volces.com/api/v3/chat/completions"
    );
}

#[test]
fn maps_local_v1_path_to_provider_base() {
    let target = build_target_url("https://example.com/v1", "v1/chat/completions", "?x=1");
    assert_eq!(target, "https://example.com/v1/chat/completions?x=1");

    let messages = build_target_url("https://example.com/v1", "v1/messages", "");
    assert_eq!(messages, "https://example.com/v1/messages");
}

#[test]
fn maps_modelscope_anthropic_base_without_v1_suffix() {
    let messages = build_target_url("https://api-inference.modelscope.cn", "v1/messages", "");
    assert_eq!(messages, "https://api-inference.modelscope.cn/v1/messages");

    let chat = build_target_url(
        "https://api-inference.modelscope.cn/v1",
        "v1/chat/completions",
        "",
    );
    assert_eq!(
        chat,
        "https://api-inference.modelscope.cn/v1/chat/completions"
    );
}

#[test]
fn converts_anthropic_messages_to_openai_chat() {
    let body = Bytes::from_static(br#"{"system":"be concise","messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}],"max_tokens":8,"temperature":0.2,"stream":true}"#);
    let payload = anthropic_to_openai(body, "target-model").unwrap();
    let value: Value = serde_json::from_slice(&payload).unwrap();

    assert_eq!(value["model"], "target-model");
    assert_eq!(value["messages"][0]["role"], "system");
    assert_eq!(value["messages"][0]["content"], "be concise");
    assert_eq!(value["messages"][1]["role"], "user");
    assert_eq!(value["messages"][1]["content"], "hello");
    assert_eq!(value["max_tokens"], 8);
    assert_eq!(value["stream"], true);
}

#[test]
fn converts_openai_chat_to_anthropic_message() {
    let response = br#"{"id":"chatcmpl-1","model":"target-model","choices":[{"message":{"role":"assistant","content":"pong"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4}}"#;
    let value = openai_json_to_anthropic_json(response).unwrap();

    assert_eq!(value["type"], "message");
    assert_eq!(value["role"], "assistant");
    assert_eq!(value["content"][0]["text"], "pong");
    assert_eq!(value["usage"]["input_tokens"], 3);
    assert_eq!(value["usage"]["output_tokens"], 4);
}

#[test]
fn free_channel_attempts_follow_failover_switch() {
    let free = crate::experimental_free::make_experimental_free_provider();
    let on = upstream_attempts(&free, true);
    assert!(on.len() > 1, "failover on → multi free routes");
    let off = upstream_attempts(&free, false);
    assert_eq!(off.len(), 1, "failover off → only selected model");
    assert_eq!(off[0].model_name, free.model_name);
}

/// 流式响应：守卫必须活到 body 读完，而不是 handler 返回就归零。
#[tokio::test]
async fn active_guard_lives_until_streaming_body_ends() {
    let counter = Arc::new(AtomicU64::new(0));
    let guard = ActiveRequestGuard::new(counter.clone());
    let chunks = vec![
        Ok::<_, std::io::Error>(Bytes::from_static(b"data: one\n\n")),
        Ok::<_, std::io::Error>(Bytes::from_static(b"data: two\n\n")),
    ];
    let body = Body::from_stream(futures_util::stream::iter(chunks));
    let response = hold_guard_through_body(Response::new(body), guard);

    // handler 已返回、body 一个字节都还没读：在途数必须仍是 1
    assert_eq!(counter.load(Ordering::Relaxed), 1, "响应体未消费时不应提前归零");

    let mut stream = response.into_body().into_data_stream();
    assert_eq!(
        stream.next().await.unwrap().unwrap(),
        Bytes::from_static(b"data: one\n\n"),
        "包装层不得改写或缓冲字节"
    );
    assert_eq!(counter.load(Ordering::Relaxed), 1, "流输出中途仍应保持在途");
    assert!(stream.next().await.is_some());
    assert!(stream.next().await.is_none());
    assert_eq!(counter.load(Ordering::Relaxed), 0, "流结束才允许归零");
}

/// 客户端断开：body 被丢弃时守卫也要释放，不能泄漏计数。
#[tokio::test]
async fn active_guard_releases_when_body_is_dropped() {
    let counter = Arc::new(AtomicU64::new(0));
    let guard = ActiveRequestGuard::new(counter.clone());
    let chunks = vec![Ok::<_, std::io::Error>(Bytes::from_static(b"pending"))];
    let response = hold_guard_through_body(
        Response::new(Body::from_stream(futures_util::stream::iter(chunks))),
        guard,
    );
    assert_eq!(counter.load(Ordering::Relaxed), 1);
    drop(response.into_body());
    assert_eq!(counter.load(Ordering::Relaxed), 0, "丢弃 body 应释放守卫");
}

#[test]
fn orders_failover_providers() {
    let mut config = AppConfig::default();
    let first = ProviderConfig::new("a", "p", "https://a/v1", "k", "m1");
    let second = ProviderConfig::new("b", "p", "https://b/v1", "k", "m2");
    config.active_provider_id = Some(second.id.clone());
    config.providers.push(first);
    config.providers.push(second);
    let providers = ordered_providers(&config);
    assert_eq!(providers[0].model_name, "m2");
    assert_eq!(providers[1].model_name, "m1");
    config.failover_enabled = false;
    let only = ordered_providers(&config);
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].model_name, "m2");
}

// ---- 策略盒子选路（R3 / T3.2 + T3.3） ----

fn route_provider(id: &str, model: &str) -> ProviderConfig {
    let mut provider = ProviderConfig::new(
        format!("厂商 {id}"),
        "openai",
        "https://example.com/v1",
        "sk-test",
        model,
    );
    provider.id = id.to_string();
    provider
}

fn route_member(
    provider_id: &str,
    model_name: &str,
    weight: u32,
    priority: u32,
) -> strategy_box::StrategyMember {
    strategy_box::StrategyMember {
        provider_id: provider_id.to_string(),
        model_name: model_name.to_string(),
        weight,
        priority,
    }
}

fn route_box(id: &str, public_id: &str, members: Vec<strategy_box::StrategyMember>) -> StrategyBox {
    StrategyBox {
        id: id.to_string(),
        name: format!("盒子 {id}"),
        public_model_id: public_id.to_string(),
        members,
        mode: strategy_box::StrategyMode::Priority,
        speed_threshold_ms: None,
        cooldown_ms: None,
        enabled: true,
    }
}

fn route_config() -> AppConfig {
    let mut config = AppConfig::default();
    config.failover_enabled = true;
    config.providers.push(route_provider("p1", "m1"));
    config.providers.push(route_provider("p2", "m2"));
    config.active_provider_id = Some("p1".to_string());
    config
}

#[test]
fn resolve_box_prefers_client_model_then_active_model() {
    let boxes = vec![
        route_box("b-active", "box-active", vec![route_member("p1", "m1", 1, 1)]),
        route_box("b-client", "box-client", vec![route_member("p2", "m2", 1, 1)]),
    ];
    // ① 请求体 model 命中优先于激活 provider 的 model_name。
    let hit = resolve_box(&boxes, "box-client", Some("box-active")).expect("client hit");
    assert_eq!(hit.id, "b-client");
    // ② 请求体未命中时看激活 provider。
    let hit = resolve_box(&boxes, "unknown", Some("box-active")).expect("active hit");
    assert_eq!(hit.id, "b-active");
    // ③ 都不命中 → None（回退 ordered_providers）。
    assert!(resolve_box(&boxes, "unknown", Some("unknown")).is_none());
    assert!(resolve_box(&boxes, "", None).is_none());

    let mut disabled = route_box("b-off", "box-off", vec![route_member("p1", "m1", 1, 1)]);
    disabled.enabled = false;
    assert!(resolve_box(&[disabled], "box-off", None).is_none());
}

#[test]
fn expand_members_skips_disabled_provider_disabled_model_and_unknown_provider() {
    let mut config = AppConfig::default();
    config.providers.push(route_provider("p1", "m1"));
    let mut stopped = route_provider("p2", "m2");
    stopped.enabled = false;
    config.providers.push(stopped);
    let registry = vec![ModelEntry {
        id: "p1::off".to_string(),
        provider_id: "p1".to_string(),
        model_name: "off".to_string(),
        enabled: false,
        last_probe: None,
    }];
    let members = vec![
        route_member("p1", "ok", 1, 1),
        route_member("p1", "off", 1, 2),
        route_member("p2", "m2", 1, 3),
        route_member("missing", "x", 1, 4),
    ];
    let expanded = expand_members(&config, &members, &registry);
    assert_eq!(expanded.len(), 1, "只应保留 p1::ok");
    assert_eq!(expanded[0].id, "p1");
    assert_eq!(expanded[0].model_name, "ok");
}

#[test]
fn member_key_of_matches_strategy_box_key() {
    let provider = route_provider("p1", "m1");
    assert_eq!(
        member_key_of("box", &provider),
        strategy_box::member_key("box", &route_member("p1", "m1", 1, 1))
    );
}

#[tokio::test]
async fn request_route_uses_box_chain_when_client_model_hits() {
    let config = route_config();
    let state = GatewayState::new(AppConfig::default(), None).expect("state");
    state
        .set_strategy_boxes(vec![route_box(
            "b1",
            "box-a",
            vec![
                route_member("p2", "m2-alt", 1, 2),
                route_member("p1", "m1-alt", 1, 1),
            ],
        )])
        .await;

    let route = request_route(&state, &config, "box-a").await;
    assert_eq!(route.box_id.as_deref(), Some("b1"));
    let chain: Vec<&str> = route
        .providers
        .iter()
        .map(|provider| provider.model_name.as_str())
        .collect();
    // priority 1 在前，成员的 model_name 覆盖 provider 自己的。
    assert_eq!(chain, vec!["m1-alt", "m2-alt"]);
}

#[tokio::test]
async fn request_route_uses_box_chain_when_active_model_hits() {
    let mut config = route_config();
    config.providers[0].model_name = "box-a".to_string();
    let state = GatewayState::new(AppConfig::default(), None).expect("state");
    state
        .set_strategy_boxes(vec![route_box(
            "b1",
            "box-a",
            vec![route_member("p2", "m2", 1, 1)],
        )])
        .await;

    let route = request_route(&state, &config, "whatever-the-client-sent").await;
    assert_eq!(route.box_id.as_deref(), Some("b1"));
    assert_eq!(route.providers.len(), 1);
    assert_eq!(route.providers[0].id, "p2");
}

#[tokio::test]
async fn request_route_without_hit_matches_ordered_providers() {
    let config = route_config();
    let state = GatewayState::new(AppConfig::default(), None).expect("state");
    state
        .set_strategy_boxes(vec![route_box(
            "b1",
            "box-a",
            vec![route_member("p2", "m2", 1, 1)],
        )])
        .await;

    let route = request_route(&state, &config, "not-a-box").await;
    assert!(route.box_id.is_none());
    assert!(route.cooldown_ms.is_none());
    let ids: Vec<&str> = route.providers.iter().map(|p| p.id.as_str()).collect();
    let expected: Vec<String> = ordered_providers(&config)
        .iter()
        .map(|provider| provider.id.clone())
        .collect();
    assert_eq!(ids, expected);
}

#[tokio::test]
async fn request_route_falls_back_when_every_member_invalid() {
    let config = route_config();
    let state = GatewayState::new(AppConfig::default(), None).expect("state");
    state
        .set_strategy_boxes(vec![route_box(
            "b1",
            "box-a",
            vec![route_member("missing", "x", 1, 1)],
        )])
        .await;

    let route = request_route(&state, &config, "box-a").await;
    assert!(route.box_id.is_none(), "空链必须回退，而不是判死请求");
    assert!(!route.providers.is_empty());
}

