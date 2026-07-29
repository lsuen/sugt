# 国内 LLM 服务商接入参考

与 `src/providerPresets.ts`、`src-tauri/src/provider_catalog.rs` 保持同步。  
SUGT 网关按 **OpenAI 兼容** 或 **Anthropic 原生** 转发；切换协议时会尽量填入官方 Base URL，未文档化的协议不自动填 URL，但**不禁止选择**。

## 字段说明

| 字段 | 说明 |
|------|------|
| OpenAI Base | `POST …/v1/chat/completions` 或厂商等价路径 |
| Anthropic Base | `POST …/v1/messages`（SUGT 会在 base 后拼接 `v1/messages`） |
| 模型列表 | 「获取模型」调用 OpenAI 风格 `GET …/models`（见下方解析规则） |

### 模型列表 URL 解析

1. base 以 `/api/v3` 结尾 → `{base}/models`（火山方舟）
2. base 以 `/v4` 或 `/paas/v4` 结尾 → `{base}/models`（智谱）
3. base 以 `/v1` 结尾 → `{base}/models`
4. 其他 → `{base}/v1/models`

Anthropic 协议下拉列表时，若厂商提供 OpenAI 列表接口，仍用 **OpenAI Base** 拉取。

---

## 服务商一览

### 魔搭社区 ModelScope

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://api-inference.modelscope.cn/v1` |
| Anthropic | `https://api-inference.modelscope.cn` |
| 模型列表 | ✅ OpenAI `/v1/models` |
| 文档 | https://modelscope.cn/docs/model-service/API-Inference/intro |

### DeepSeek

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://api.deepseek.com/v1` |
| Anthropic | `https://api.deepseek.com/anthropic` |
| 模型列表 | ✅ |
| 文档 | https://api-docs.deepseek.com/guides/anthropic_api |

### 通义千问 · DashScope

| 协议 | Base URL |
|------|----------|
| OpenAI 兼容 | `https://dashscope.aliyuncs.com/compatible-mode/v1` |
| Anthropic | `https://dashscope.aliyuncs.com/apps/anthropic` |
| 模型列表 | ✅（兼容模式） |
| 文档 | https://help.aliyun.com/zh/model-studio/anthropic-api-messages |

Coding Plan 等套餐另有专属域名，需在控制台确认后手动改 Base URL。

### Moonshot · Kimi

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://api.moonshot.cn/v1` |
| Anthropic | `https://api.moonshot.cn/anthropic` |
| 模型列表 | ✅ |
| 文档 | https://platform.moonshot.cn/docs |

### 智谱 AI

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://open.bigmodel.cn/api/paas/v4` |
| Anthropic | `https://open.bigmodel.cn/api/anthropic` |
| 模型列表 | ✅ `/api/paas/v4/models` |
| 文档 | https://docs.bigmodel.cn/cn/guide/develop/claude/introduction |

### 火山引擎 · 方舟

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://ark.cn-beijing.volces.com/api/v3` |
| Anthropic | `https://ark.cn-beijing.volces.com/api/coding` |
| 模型列表 | ✅ `/api/v3/models`（接入点 ID 作 model） |
| 文档 | https://www.volcengine.com/docs/82379 |

Coding Plan OpenAI：`https://ark.cn-beijing.volces.com/api/coding/v3`（需在预设外手动配置）。

### 硅基流动 SiliconFlow

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://api.siliconflow.cn/v1` |
| Anthropic | `https://api.siliconflow.cn` |
| 模型列表 | ✅ |
| 文档 | https://docs.siliconflow.cn/ |

### MiniMax

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://api.minimaxi.com/v1` |
| Anthropic | `https://api.minimaxi.com/anthropic` |
| 模型列表 | ✅ |
| 文档 | https://platform.minimaxi.com/docs |

### 百川智能

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://api.baichuan-ai.com/v1` |
| Anthropic | 未文档化，可选手填 |
| 模型列表 | ❌ 手填 |
| 文档 | https://platform.baichuan-ai.com/docs |

### 阶跃星辰 StepFun

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://api.stepfun.com/v1` |
| Anthropic | 未文档化，可选手填 |
| 模型列表 | ✅（以官方为准） |
| 文档 | https://platform.stepfun.com/docs |

### 零一万物 Yi

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://api.lingyiwanwu.com/v1` |
| Anthropic | 未文档化，可选手填 |
| 模型列表 | ✅（以官方为准） |
| 文档 | https://platform.lingyiwanwu.com/docs |

### 腾讯混元

| 协议 | Base URL |
|------|----------|
| OpenAI | `https://api.hunyuan.cloud.tencent.com/v1` |
| Anthropic | 未文档化，可选手填 |
| 模型列表 | ❌ 手填（如 `hunyuan-turbos-latest`） |
| 文档 | https://cloud.tencent.com/document/product/1729/111007 |

---

## 维护说明

1. 更新厂商信息时同时改：`providerPresets.ts`、`provider_catalog.rs`、本文件。
2. 新增厂商：在 `VENDOR_DEFINITIONS` 增加一项，并在 Rust `VENDORS` 镜像。
3. 不要在前端 `disabled` 协议选项；未知 Anthropic 地址时留空 Base URL 并提示用户查阅文档。
