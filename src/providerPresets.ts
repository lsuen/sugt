export type ProviderProtocol = 'openai' | 'anthropic';

export type ProviderForm = {
  id?: string;
  name: string;
  provider: string;
  base_url: string;
  api_key: string;
  model_name: string;
  protocol: ProviderProtocol;
  enabled: boolean;
  /** 保存时自动规范化 Base URL；关闭则原样保存 */
  auto_adapt_base_url: boolean;
};

export type TrafficStats = {
  total_requests: number;
  today_requests: number;
  success_count: number;
  failed_count: number;
  avg_latency_ms: number;
  fail_rate_percent: number;
  active_clients: number;
  total_input_tokens: number;
  total_output_tokens: number;
  today_input_tokens: number;
  today_output_tokens: number;
  hourly_buckets: { success: number; failed: number }[];
  recent_clients: {
    label: string;
    provider_name: string;
    model_name: string;
    request_count: number;
    last_path: string;
    last_at: string;
  }[];
};

export type ModelsListMode = 'openai_compatible' | 'manual';

/** 国内服务商定义（详见 docs/provider-vendors.md） */
export type VendorDefinition = {
  id: string;
  label: string;
  provider: string;
  openaiBaseUrl: string;
  /** 无官方文档时不填；切换 Anthropic 时不会自动覆盖已有手填 URL */
  anthropicBaseUrl?: string;
  defaultModel?: string;
  modelsListMode: ModelsListMode;
  docUrl: string;
  notes?: string;
};

export const VENDOR_DEFINITIONS: VendorDefinition[] = [
  {
    id: 'modelscope',
    label: '魔搭社区 ModelScope',
    provider: 'ModelScope',
    openaiBaseUrl: 'https://api-inference.modelscope.cn/v1',
    anthropicBaseUrl: 'https://api-inference.modelscope.cn',
    modelsListMode: 'openai_compatible',
    docUrl: 'https://modelscope.cn/docs/model-service/API-Inference/intro',
  },
  {
    id: 'deepseek',
    label: 'DeepSeek 官方',
    provider: 'DeepSeek',
    openaiBaseUrl: 'https://api.deepseek.com/v1',
    anthropicBaseUrl: 'https://api.deepseek.com/anthropic',
    defaultModel: 'deepseek-chat',
    modelsListMode: 'openai_compatible',
    docUrl: 'https://api-docs.deepseek.com/guides/anthropic_api',
  },
  {
    id: 'dashscope',
    label: '通义千问 · 阿里云 DashScope',
    provider: 'Alibaba',
    openaiBaseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    anthropicBaseUrl: 'https://dashscope.aliyuncs.com/apps/anthropic',
    defaultModel: 'qwen-plus',
    modelsListMode: 'openai_compatible',
    docUrl: 'https://help.aliyun.com/zh/model-studio/anthropic-api-messages',
    notes: 'Coding Plan 等套餐可能有专属域名，请以控制台为准',
  },
  {
    id: 'moonshot',
    label: 'Moonshot · Kimi',
    provider: 'Moonshot',
    openaiBaseUrl: 'https://api.moonshot.cn/v1',
    anthropicBaseUrl: 'https://api.moonshot.cn/anthropic',
    defaultModel: 'moonshot-v1-8k',
    modelsListMode: 'openai_compatible',
    docUrl: 'https://platform.moonshot.cn/docs',
  },
  {
    id: 'zhipu',
    label: '智谱 AI · GLM',
    provider: 'Zhipu',
    openaiBaseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    anthropicBaseUrl: 'https://open.bigmodel.cn/api/anthropic',
    defaultModel: 'glm-4-flash',
    modelsListMode: 'openai_compatible',
    docUrl: 'https://docs.bigmodel.cn/cn/guide/develop/claude/introduction',
  },
  {
    id: 'volcengine',
    label: '火山引擎 · 方舟',
    provider: 'Volcengine',
    openaiBaseUrl: 'https://ark.cn-beijing.volces.com/api/coding/v3',
    anthropicBaseUrl: 'https://ark.cn-beijing.volces.com/api/coding',
    modelsListMode: 'openai_compatible',
    docUrl: 'https://www.volcengine.com/docs/82379',
    notes: 'Coding Plan OpenAI 默认 /api/coding/v3；方舟标准接入可用 /api/v3',
  },
  {
    id: 'siliconflow',
    label: '硅基流动 SiliconFlow',
    provider: 'SiliconFlow',
    openaiBaseUrl: 'https://api.siliconflow.cn/v1',
    anthropicBaseUrl: 'https://api.siliconflow.cn',
    modelsListMode: 'openai_compatible',
    docUrl: 'https://docs.siliconflow.cn/',
  },
  {
    id: 'minimax',
    label: 'MiniMax',
    provider: 'MiniMax',
    openaiBaseUrl: 'https://api.minimaxi.com/v1',
    anthropicBaseUrl: 'https://api.minimaxi.com/anthropic',
    defaultModel: 'MiniMax-M2.5',
    modelsListMode: 'openai_compatible',
    docUrl: 'https://platform.minimaxi.com/docs',
  },
  {
    id: 'baichuan',
    label: '百川智能',
    provider: 'Baichuan',
    openaiBaseUrl: 'https://api.baichuan-ai.com/v1',
    defaultModel: 'Baichuan4-Turbo',
    modelsListMode: 'manual',
    docUrl: 'https://platform.baichuan-ai.com/docs',
    notes: '暂无公开模型列表接口，请手填 Model Name',
  },
  {
    id: 'stepfun',
    label: '阶跃星辰 StepFun',
    provider: 'StepFun',
    openaiBaseUrl: 'https://api.stepfun.com/v1',
    defaultModel: 'step-1-8k',
    modelsListMode: 'openai_compatible',
    docUrl: 'https://platform.stepfun.com/docs',
  },
  {
    id: 'lingyi',
    label: '零一万物 Yi',
    provider: '01.AI',
    openaiBaseUrl: 'https://api.lingyiwanwu.com/v1',
    defaultModel: 'yi-lightning',
    modelsListMode: 'openai_compatible',
    docUrl: 'https://platform.lingyiwanwu.com/docs',
  },
  {
    id: 'tencent',
    label: '腾讯混元',
    provider: 'Tencent',
    openaiBaseUrl: 'https://api.hunyuan.cloud.tencent.com/v1',
    defaultModel: 'hunyuan-turbos-latest',
    modelsListMode: 'manual',
    docUrl: 'https://cloud.tencent.com/document/product/1729/111007',
    notes: '暂无模型列表接口，请手填如 hunyuan-turbos-latest',
  },
];

export function vendorBaseUrl(
  vendor: VendorDefinition,
  protocol: ProviderProtocol,
): string | undefined {
  if (protocol === 'anthropic') {
    return vendor.anthropicBaseUrl;
  }
  return vendor.openaiBaseUrl;
}

export function applyVendorToForm(
  vendor: VendorDefinition,
  protocol: ProviderProtocol,
  prev: ProviderForm,
): ProviderForm {
  const suggested = vendorBaseUrl(vendor, protocol);
  const base_url = suggested ?? prev.base_url;
  const model_name =
    vendor.defaultModel !== undefined && vendor.defaultModel !== ''
      ? vendor.defaultModel
      : prev.model_name;
  return {
    ...prev,
    name: prev.name || vendor.label,
    provider: vendor.provider,
    base_url,
    model_name,
    protocol,
  };
}

export function findVendorById(id: string): VendorDefinition | undefined {
  return VENDOR_DEFINITIONS.find((v) => v.id === id);
}

export function guessVendorFromForm(form: ProviderForm): string {
  const base = form.base_url.trim().toLowerCase();
  const hit = VENDOR_DEFINITIONS.find((v) => {
    const openai = v.openaiBaseUrl.toLowerCase();
    const anthropic = v.anthropicBaseUrl?.toLowerCase();
    return (
      base === openai ||
      base === openai.replace(/\/v1$/, '') ||
      (anthropic && (base === anthropic || base.startsWith(anthropic))) ||
      base.includes(openai.replace(/\/v1$/, '').replace('https://', ''))
    );
  });
  return hit?.id ?? '';
}

export function vendorSupportsModelList(vendor: VendorDefinition): boolean {
  return vendor.modelsListMode === 'openai_compatible';
}
