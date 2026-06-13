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

/** 国内服务商定义：协议切换时自动匹配官方 Base URL */
export type VendorDefinition = {
  id: string;
  label: string;
  provider: string;
  openaiBaseUrl: string;
  anthropicBaseUrl: string;
  /** 统一调度/默认模型（留空则需用户填写或拉取列表） */
  defaultModel?: string;
  anthropicSupported: boolean;
};

export const VENDOR_DEFINITIONS: VendorDefinition[] = [
  {
    id: 'modelscope',
    label: '魔搭社区 ModelScope',
    provider: 'ModelScope',
    openaiBaseUrl: 'https://api-inference.modelscope.cn/v1',
    anthropicBaseUrl: 'https://api-inference.modelscope.cn',
    defaultModel: '',
    anthropicSupported: true,
  },
  {
    id: 'deepseek',
    label: 'DeepSeek 官方',
    provider: 'DeepSeek',
    openaiBaseUrl: 'https://api.deepseek.com/v1',
    anthropicBaseUrl: 'https://api.deepseek.com',
    defaultModel: 'deepseek-chat',
    anthropicSupported: false,
  },
  {
    id: 'dashscope',
    label: '通义千问 · 阿里云 DashScope',
    provider: 'Alibaba',
    openaiBaseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    anthropicBaseUrl: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    defaultModel: 'qwen-plus',
    anthropicSupported: false,
  },
  {
    id: 'moonshot',
    label: 'Moonshot · Kimi',
    provider: 'Moonshot',
    openaiBaseUrl: 'https://api.moonshot.cn/v1',
    anthropicBaseUrl: 'https://api.moonshot.cn/v1',
    defaultModel: 'moonshot-v1-8k',
    anthropicSupported: false,
  },
  {
    id: 'zhipu',
    label: '智谱 AI · GLM',
    provider: 'Zhipu',
    openaiBaseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    anthropicBaseUrl: 'https://open.bigmodel.cn/api/paas/v4',
    defaultModel: 'glm-4-flash',
    anthropicSupported: false,
  },
  {
    id: 'volcengine',
    label: '火山引擎 · 方舟',
    provider: 'Volcengine',
    openaiBaseUrl: 'https://ark.cn-beijing.volces.com/api/v3',
    anthropicBaseUrl: 'https://ark.cn-beijing.volces.com/api/v3',
    defaultModel: '',
    anthropicSupported: false,
  },
  {
    id: 'siliconflow',
    label: '硅基流动 SiliconFlow',
    provider: 'SiliconFlow',
    openaiBaseUrl: 'https://api.siliconflow.cn/v1',
    anthropicBaseUrl: 'https://api.siliconflow.cn/v1',
    defaultModel: '',
    anthropicSupported: false,
  },
  {
    id: 'minimax',
    label: 'MiniMax',
    provider: 'MiniMax',
    openaiBaseUrl: 'https://api.minimax.chat/v1',
    anthropicBaseUrl: 'https://api.minimax.chat/v1',
    defaultModel: 'abab6.5s-chat',
    anthropicSupported: false,
  },
  {
    id: 'baichuan',
    label: '百川智能',
    provider: 'Baichuan',
    openaiBaseUrl: 'https://api.baichuan-ai.com/v1',
    anthropicBaseUrl: 'https://api.baichuan-ai.com/v1',
    defaultModel: 'Baichuan4-Turbo',
    anthropicSupported: false,
  },
  {
    id: 'stepfun',
    label: '阶跃星辰 StepFun',
    provider: 'StepFun',
    openaiBaseUrl: 'https://api.stepfun.com/v1',
    anthropicBaseUrl: 'https://api.stepfun.com/v1',
    defaultModel: 'step-1-8k',
    anthropicSupported: false,
  },
  {
    id: 'lingyi',
    label: '零一万物 Yi',
    provider: '01.AI',
    openaiBaseUrl: 'https://api.lingyiwanwu.com/v1',
    anthropicBaseUrl: 'https://api.lingyiwanwu.com/v1',
    defaultModel: 'yi-lightning',
    anthropicSupported: false,
  },
  {
    id: 'tencent',
    label: '腾讯混元',
    provider: 'Tencent',
    openaiBaseUrl: 'https://api.hunyuan.cloud.tencent.com/v1',
    anthropicBaseUrl: 'https://api.hunyuan.cloud.tencent.com/v1',
    defaultModel: 'hunyuan-lite',
    anthropicSupported: false,
  },
];

export function vendorBaseUrl(vendor: VendorDefinition, protocol: ProviderProtocol): string {
  if (protocol === 'anthropic' && vendor.anthropicSupported) {
    return vendor.anthropicBaseUrl;
  }
  return vendor.openaiBaseUrl;
}

export function applyVendorToForm(
  vendor: VendorDefinition,
  protocol: ProviderProtocol,
  prev: ProviderForm,
): ProviderForm {
  const base_url = vendorBaseUrl(vendor, protocol);
  const model_name = vendor.defaultModel ?? prev.model_name;
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
  const hit = VENDOR_DEFINITIONS.find(
    (v) =>
      base.includes(v.openaiBaseUrl.toLowerCase().replace(/\/v1$/, '')) ||
      base === v.openaiBaseUrl.toLowerCase() ||
      base === v.anthropicBaseUrl.toLowerCase(),
  );
  return hit?.id ?? '';
}
