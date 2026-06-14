//! 国内 LLM 服务商接入目录（与 `docs/provider-vendors.md` 同步）

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelsListMode {
    /// GET OpenAI 风格 /v1/models（或按 base 智能解析）
    OpenAiCompatible,
    /// 无公开列表接口，需手填模型 ID
    Manual,
}

#[derive(Debug, Clone)]
pub struct VendorCatalogEntry {
    pub id: &'static str,
    pub openai_base_url: &'static str,
    pub anthropic_base_url: Option<&'static str>,
    pub models_list_mode: ModelsListMode,
    pub doc_url: &'static str,
}

pub fn vendor_by_id(id: &str) -> Option<&'static VendorCatalogEntry> {
    VENDORS.iter().find(|v| v.id == id)
}

/// 解析模型列表请求 URL（兼容 /v1、/api/v3、/paas/v4 等常见形态）
pub fn resolve_models_list_url(base_url: &str) -> String {
    let base = base_url.trim().trim_end_matches('/');
    if base.ends_with("/api/v3") {
        return format!("{}/models", base);
    }
    if base.ends_with("/v4") || base.ends_with("/paas/v4") {
        return format!("{}/models", base);
    }
    if base.ends_with("/v1") {
        return format!("{}/models", base);
    }
    format!("{}/v1/models", base)
}

pub fn models_list_base_for_vendor(entry: &VendorCatalogEntry) -> Option<&'static str> {
    match entry.models_list_mode {
        ModelsListMode::OpenAiCompatible => Some(entry.openai_base_url),
        ModelsListMode::Manual => None,
    }
}

pub static VENDORS: [VendorCatalogEntry; 12] = [
    VendorCatalogEntry {
        id: "modelscope",
        openai_base_url: "https://api-inference.modelscope.cn/v1",
        anthropic_base_url: Some("https://api-inference.modelscope.cn"),
        models_list_mode: ModelsListMode::OpenAiCompatible,
        doc_url: "https://modelscope.cn/docs/model-service/API-Inference/intro",
    },
    VendorCatalogEntry {
        id: "deepseek",
        openai_base_url: "https://api.deepseek.com/v1",
        anthropic_base_url: Some("https://api.deepseek.com/anthropic"),
        models_list_mode: ModelsListMode::OpenAiCompatible,
        doc_url: "https://api-docs.deepseek.com/",
    },
    VendorCatalogEntry {
        id: "dashscope",
        openai_base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        anthropic_base_url: Some("https://dashscope.aliyuncs.com/apps/anthropic"),
        models_list_mode: ModelsListMode::OpenAiCompatible,
        doc_url: "https://help.aliyun.com/zh/model-studio/anthropic-api-messages",
    },
    VendorCatalogEntry {
        id: "moonshot",
        openai_base_url: "https://api.moonshot.cn/v1",
        anthropic_base_url: Some("https://api.moonshot.cn/anthropic"),
        models_list_mode: ModelsListMode::OpenAiCompatible,
        doc_url: "https://platform.moonshot.cn/docs",
    },
    VendorCatalogEntry {
        id: "zhipu",
        openai_base_url: "https://open.bigmodel.cn/api/paas/v4",
        anthropic_base_url: Some("https://open.bigmodel.cn/api/anthropic"),
        models_list_mode: ModelsListMode::OpenAiCompatible,
        doc_url: "https://docs.bigmodel.cn/cn/guide/develop/claude/introduction",
    },
    VendorCatalogEntry {
        id: "volcengine",
        openai_base_url: "https://ark.cn-beijing.volces.com/api/v3",
        anthropic_base_url: Some("https://ark.cn-beijing.volces.com/api/coding"),
        models_list_mode: ModelsListMode::OpenAiCompatible,
        doc_url: "https://www.volcengine.com/docs/82379",
    },
    VendorCatalogEntry {
        id: "siliconflow",
        openai_base_url: "https://api.siliconflow.cn/v1",
        anthropic_base_url: Some("https://api.siliconflow.cn"),
        models_list_mode: ModelsListMode::OpenAiCompatible,
        doc_url: "https://docs.siliconflow.cn/",
    },
    VendorCatalogEntry {
        id: "minimax",
        openai_base_url: "https://api.minimaxi.com/v1",
        anthropic_base_url: Some("https://api.minimaxi.com/anthropic"),
        models_list_mode: ModelsListMode::OpenAiCompatible,
        doc_url: "https://platform.minimaxi.com/docs",
    },
    VendorCatalogEntry {
        id: "baichuan",
        openai_base_url: "https://api.baichuan-ai.com/v1",
        anthropic_base_url: None,
        models_list_mode: ModelsListMode::Manual,
        doc_url: "https://platform.baichuan-ai.com/docs",
    },
    VendorCatalogEntry {
        id: "stepfun",
        openai_base_url: "https://api.stepfun.com/v1",
        anthropic_base_url: None,
        models_list_mode: ModelsListMode::OpenAiCompatible,
        doc_url: "https://platform.stepfun.com/docs",
    },
    VendorCatalogEntry {
        id: "lingyi",
        openai_base_url: "https://api.lingyiwanwu.com/v1",
        anthropic_base_url: None,
        models_list_mode: ModelsListMode::OpenAiCompatible,
        doc_url: "https://platform.lingyiwanwu.com/docs",
    },
    VendorCatalogEntry {
        id: "tencent",
        openai_base_url: "https://api.hunyuan.cloud.tencent.com/v1",
        anthropic_base_url: None,
        models_list_mode: ModelsListMode::Manual,
        doc_url: "https://cloud.tencent.com/document/product/1729/111007",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_volcengine_v3_models_url() {
        let url = resolve_models_list_url("https://ark.cn-beijing.volces.com/api/v3");
        assert_eq!(url, "https://ark.cn-beijing.volces.com/api/v3/models");
    }

    #[test]
    fn resolves_zhipu_v4_models_url() {
        let url = resolve_models_list_url("https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(url, "https://open.bigmodel.cn/api/paas/v4/models");
    }

    #[test]
    fn volcengine_has_anthropic_base() {
        let v = vendor_by_id("volcengine").unwrap();
        assert_eq!(
            v.anthropic_base_url,
            Some("https://ark.cn-beijing.volces.com/api/coding")
        );
    }
}
