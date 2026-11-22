//! 配置预设端到端回归。
//!
//! 每个用例使用独立临时目录并直接构造 `AppRuntime`（与 api_key_pool_e2e.rs 同思路），
//! 绝不触碰真实 config_dir，也不依赖进程级 SUGT_CONFIG_DIR，避免并行用例互相污染。
//! 覆盖：内置三档列出、apply 只改覆盖字段、save/delete/export/import 往返、
//! 删内置被拒、import 强制改 id 防覆盖、导出无敏感串。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use sugt_lib::commands::AppRuntime;
use sugt_lib::config::{load_config_file, AppPaths};
use sugt_lib::gateway::GatewayState;
use sugt_lib::model::{AppConfig, ProviderConfig};
use sugt_lib::presets;
use tokio::sync::RwLock;

/// 独立临时目录：pid + 纳秒，Windows 也安全。
fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("sugt-presets-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("临时目录可创建");
    dir
}

fn test_runtime(dir: &PathBuf, config: AppConfig) -> AppRuntime {
    let paths = AppPaths {
        config_dir: dir.clone(),
        config_file: dir.join("config.toml"),
        log_file: dir.join("sugt.log"),
    };
    let gateway = GatewayState::new(config.clone(), Some(paths.config_file.clone()))
        .expect("网关状态可构造");
    AppRuntime {
        paths,
        gateway,
        config: Arc::new(RwLock::new(config)),
    }
}

/// 带一个真实渠道与非默认"无关字段"的基准配置，用于验证 apply 不改其它字段。
fn base_config() -> AppConfig {
    let mut provider = ProviderConfig::new(
        "mock",
        "openai",
        "http://127.0.0.1:1/v1",
        "test-api-key-do-not-export",
        "mock-model",
    );
    provider.id = "p1".into();
    AppConfig {
        host: "192.0.2.10".into(),
        port: 9876,
        log_retention_days: 3,
        webui_port: 9999,
        providers: vec![provider],
        active_provider_id: Some("p1".into()),
        failover_enabled: false,
        upstream_retry_budget_ms: None,
        upstream_retry_single_ms: None,
        experimental_free_idle_probe_enabled: true,
        experimental_free_auto_rotate: true,
        ..Default::default()
    }
}

#[test]
fn builtin_presets_are_listed_first_with_expected_settings() {
    let dir = temp_dir("list");
    let list = presets::list_config_presets_core(&dir).expect("内置预设可列出");

    assert_eq!(list.len(), 3, "初始只有三档内置预设");
    assert_eq!(
        list.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        vec!["builtin-work", "builtin-save", "builtin-turbo"]
    );
    assert!(list.iter().all(|p| p.builtin), "三档均为内置");
    assert!(list.iter().all(|p| !p.updated_at.is_empty()), "时间戳非空");

    let work = &list[0];
    assert!(work.settings.failover_enabled, "稳定工作档默认开启 failover");
    assert_eq!(work.settings.upstream_retry_budget_ms, Some(480_000));
    assert_eq!(work.settings.upstream_retry_single_ms, Some(60_000));
    assert!(!work.settings.experimental_free_idle_probe_enabled);
    assert!(!work.settings.experimental_free_auto_rotate);
    assert_eq!(work.settings.active_provider_id, None, "内置档不指定模型");

    assert!(list[1].settings.experimental_free_idle_probe_enabled);
    assert!(list[1].settings.experimental_free_auto_rotate, "省钱档自动轮换");
    assert!(!list[2].settings.failover_enabled, "极速档关闭 failover");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn apply_overrides_only_preset_fields_and_persists_to_config_toml() {
    let dir = temp_dir("apply");
    let runtime = test_runtime(&dir, base_config());

    let applied = presets::apply_config_preset_core(&runtime, "builtin-work")
        .await
        .expect("可应用稳定工作档");
    assert_eq!(applied.id, "builtin-work");

    {
        let config = runtime.config.read().await;
        assert!(config.failover_enabled, "failover 被预设改为 true");
        assert_eq!(config.upstream_retry_budget_ms, Some(480_000));
        assert_eq!(config.upstream_retry_single_ms, Some(60_000));
        assert!(!config.experimental_free_idle_probe_enabled);
        assert!(!config.experimental_free_auto_rotate);
        assert_eq!(
            config.active_provider_id.as_deref(),
            Some("p1"),
            "activeProviderId=None 时不得改模型"
        );
        // 预设未覆盖的字段原样保留。
        assert_eq!(config.host, "192.0.2.10");
        assert_eq!(config.port, 9876);
        assert_eq!(config.log_retention_days, 3);
        assert_eq!(config.webui_port, 9999);
        assert_eq!(config.providers.len(), 1);
    }

    // 磁盘上的 config.toml 必须同步到相同值，且无关字段不变。
    let (disk, _) = load_config_file(&dir.join("config.toml")).expect("config.toml 可解析");
    assert!(disk.failover_enabled);
    assert_eq!(disk.upstream_retry_budget_ms, Some(480_000));
    assert_eq!(disk.upstream_retry_single_ms, Some(60_000));
    assert!(!disk.experimental_free_idle_probe_enabled);
    assert!(!disk.experimental_free_auto_rotate);
    assert_eq!(disk.host, "192.0.2.10");
    assert_eq!(disk.port, 9876);
    assert_eq!(disk.log_retention_days, 3);
    assert_eq!(disk.webui_port, 9999);

    // 再切极速档：failover 回到 false，模型仍不变。
    presets::apply_config_preset_core(&runtime, "builtin-turbo")
        .await
        .expect("可应用极速档");
    let config = runtime.config.read().await;
    assert!(!config.failover_enabled);
    assert_eq!(config.active_provider_id.as_deref(), Some("p1"));
    drop(config);

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn save_delete_export_import_roundtrip() {
    let dir = temp_dir("roundtrip");
    let runtime = test_runtime(&dir, base_config());

    // save：去空白、生成 custom id、快照当前配置。
    let saved = presets::save_config_preset_core(&runtime, "  我的配置  ")
        .await
        .expect("可保存自定义预设");
    assert_eq!(saved.name, "我的配置");
    assert!(saved.id.starts_with("custom-"), "自定义 id 格式");
    assert!(!saved.builtin);
    assert!(!saved.settings.failover_enabled);
    assert_eq!(saved.settings.active_provider_id.as_deref(), Some("p1"));
    assert!(saved.settings.experimental_free_idle_probe_enabled);
    assert!(saved.settings.experimental_free_auto_rotate);

    let list = presets::list_config_presets_core(&dir).expect("可列出");
    assert_eq!(list.len(), 4, "三档内置 + 一个自定义");
    assert_eq!(list[3].id, saved.id, "自定义预设排在内置之后");

    // 同名自定义预设 = upsert：保留 id、刷新快照，不新增条目。
    {
        let mut runtime = runtime.config.write().await;
        runtime.failover_enabled = true;
    }
    let overwritten = presets::save_config_preset_core(&runtime, "我的配置")
        .await
        .expect("同名自定义预设应 upsert 覆盖");
    assert_eq!(overwritten.id, saved.id, "覆盖必须保留原 id");
    assert_eq!(overwritten.name, "我的配置");
    assert!(!overwritten.builtin);
    assert!(overwritten.settings.failover_enabled, "覆盖应写入当前配置快照");
    let list = presets::list_config_presets_core(&dir).expect("可列出");
    assert_eq!(list.len(), 4, "覆盖不得新增预设");
    // 恢复基线，保证后续 export/import 断言仍以原快照为准。
    {
        let mut runtime = runtime.config.write().await;
        runtime.failover_enabled = false;
    }
    presets::save_config_preset_core(&runtime, "我的配置")
        .await
        .expect("可再次覆盖回原快照");

    // 内置档同名仍被拒，防止生成"影子内置档"。
    assert!(
        presets::save_config_preset_core(&runtime, "稳定工作")
            .await
            .is_err(),
        "内置档名称不得用作自定义预设名"
    );
    assert!(
        presets::save_config_preset_core(&runtime, "   ")
            .await
            .is_err(),
        "空白名称必须报错"
    );

    // export：JSON 不含任何敏感字段，且可反序列化回同一份设置。
    let exported = presets::export_config_preset_core(&dir, &saved.id).expect("可导出");
    for marker in [
        "apiKey",
        "api_key",
        "baseUrl",
        "base_url",
        "authorization",
        "secret",
        "test-api-key-do-not-export",
    ] {
        assert!(
            !exported.to_ascii_lowercase().contains(&marker.to_ascii_lowercase()),
            "导出内容不得包含 {marker}：{exported}"
        );
    }
    let parsed: Value = serde_json::from_str(&exported).expect("导出是合法 JSON");
    assert_eq!(parsed["settings"]["activeProviderId"], json!("p1"));
    let via_json: sugt_lib::presets::ConfigPreset =
        serde_json::from_str(&exported).expect("可反序列化回 ConfigPreset");
    assert_eq!(via_json.id, saved.id);
    assert_eq!(via_json.settings.failover_enabled, saved.settings.failover_enabled);
    assert_eq!(
        via_json.settings.upstream_retry_budget_ms,
        saved.settings.upstream_retry_budget_ms
    );

    // import：改名后导入，得到全新的 custom id。
    let mut imported_json: Value = serde_json::from_str(&exported).unwrap();
    imported_json["name"] = json!("导入的配置");
    let imported = presets::import_config_preset_core(&dir, &imported_json.to_string())
        .expect("可导入");
    assert_ne!(imported.id, saved.id, "导入必须生成新的 id");
    assert!(imported.id.starts_with("custom-"));
    assert!(!imported.builtin);
    assert_eq!(imported.name, "导入的配置");
    assert_eq!(imported.settings.failover_enabled, saved.settings.failover_enabled);
    assert_eq!(
        imported.settings.active_provider_id,
        saved.settings.active_provider_id
    );

    let list = presets::list_config_presets_core(&dir).expect("可列出");
    assert_eq!(list.len(), 5, "导入后共 5 个预设");

    // delete：只允许删自定义。
    presets::delete_config_preset_core(&dir, &saved.id).expect("可删自定义预设");
    let list = presets::list_config_presets_core(&dir).expect("可列出");
    assert_eq!(list.len(), 4);
    assert!(!list.iter().any(|p| p.id == saved.id));
    assert!(
        presets::delete_config_preset_core(&dir, "builtin-work").is_err(),
        "内置预设不可删除"
    );
    assert!(
        presets::delete_config_preset_core(&dir, "custom-not-exist").is_err(),
        "不存在的预设删除应报错"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_rewrites_id_and_rejects_invalid_input() {
    let dir = temp_dir("import");

    // 用内置档导出内容伪装成内置 id + builtin=true 导入，id 必须被重写。
    let exported = presets::export_config_preset_core(&dir, "builtin-work").expect("内置可导出");
    let mut spoof: Value = serde_json::from_str(&exported).unwrap();
    spoof["id"] = json!("builtin-work");
    spoof["builtin"] = json!(true);
    spoof["name"] = json!("伪装档");
    spoof["settings"]["failoverEnabled"] = json!(false);
    let imported = presets::import_config_preset_core(&dir, &spoof.to_string()).expect("可导入");
    assert_ne!(imported.id, "builtin-work", "id 必须重写，不得覆盖内置档");
    assert!(imported.id.starts_with("custom-"));
    assert!(!imported.builtin, "导入一律视为自定义");

    // 内置档仍以代码为准，不受伪装条目影响。
    let list = presets::list_config_presets_core(&dir).expect("可列出");
    assert_eq!(list.len(), 4);
    let work = list.iter().find(|p| p.id == "builtin-work").unwrap();
    assert!(work.builtin);
    assert!(work.settings.failover_enabled, "仍是内置档原始设置");

    // 各种非法输入明确报错，不吞异常。
    assert!(presets::import_config_preset_core(&dir, "not json").is_err());
    let mut empty_name: Value = serde_json::from_str(&exported).unwrap();
    empty_name["name"] = json!("   ");
    assert!(presets::import_config_preset_core(&dir, &empty_name.to_string()).is_err());
    let mut zero_budget: Value = serde_json::from_str(&exported).unwrap();
    zero_budget["settings"]["upstreamRetryBudgetMs"] = json!(0);
    assert!(presets::import_config_preset_core(&dir, &zero_budget.to_string()).is_err());
    let mut huge_budget: Value = serde_json::from_str(&exported).unwrap();
    huge_budget["settings"]["upstreamRetryBudgetMs"] = json!(86_400_001u64);
    assert!(presets::import_config_preset_core(&dir, &huge_budget.to_string()).is_err());

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_refuses_content_with_sensitive_field_names() {
    let dir = temp_dir("sensitive");
    // 直接篡改持久化文件，模拟未来误把敏感字段塞进预设结构的回归。
    let forged = json!([{
        "id": "custom-forged",
        "name": "api_key 测试",
        "builtin": false,
        "settings": {
            "activeProviderId": null,
            "failoverEnabled": true,
            "upstreamRetryBudgetMs": 480000,
            "upstreamRetrySingleMs": 60000,
            "experimentalFreeIdleProbeEnabled": false,
            "experimentalFreeAutoRotate": false
        },
        "updatedAt": "2026-01-01T00:00:00Z"
    }]);
    std::fs::write(dir.join("config-presets.json"), forged.to_string()).expect("可写预设文件");

    let err = presets::export_config_preset_core(&dir, "custom-forged")
        .expect_err("含敏感字段的预设导出必须被拒");
    assert!(err.contains("敏感字段"), "错误信息应明确：{err}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn apply_rejects_unknown_provider_id() {
    let dir = temp_dir("provider");
    let runtime = test_runtime(&dir, base_config());

    // 直接写入一个指定了不存在渠道的预设。
    let forged = json!([{
        "id": "custom-bad-provider",
        "name": "坏渠道",
        "builtin": false,
        "settings": {
            "activeProviderId": "missing-provider",
            "failoverEnabled": true,
            "upstreamRetryBudgetMs": 480000,
            "upstreamRetrySingleMs": 60000,
            "experimentalFreeIdleProbeEnabled": false,
            "experimentalFreeAutoRotate": false
        },
        "updatedAt": "2026-01-01T00:00:00Z"
    }]);
    std::fs::write(dir.join("config-presets.json"), forged.to_string()).expect("可写预设文件");

    let err = presets::apply_config_preset_core(&runtime, "custom-bad-provider")
        .await
        .expect_err("未知模型渠道必须拒绝");
    assert!(err.contains("模型渠道不存在"), "错误信息应明确：{err}");

    // 被拒后配置没有被改动。
    let config = runtime.config.read().await;
    assert!(!config.failover_enabled, "拒绝应用时不得写配置");
    drop(config);

    let _ = std::fs::remove_dir_all(&dir);
}
