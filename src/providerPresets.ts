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

export type ProviderPreset = {
  id: string;
  label: string;
  form: Partial<ProviderForm>;
};

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: 'modelscope-deepseek',
    label: '魔搭 · DeepSeek V4 Flash',
    form: {
      name: '魔搭 DeepSeek',
      provider: 'ModelScope',
      base_url: 'https://api-inference.modelscope.cn/v1',
      model_name: 'deepseek-ai/DeepSeek-V4-Flash',
      protocol: 'openai',
    },
  },
  {
    id: 'modelscope-qwen',
    label: '魔搭 · Qwen3',
    form: {
      name: '魔搭 Qwen',
      provider: 'ModelScope',
      base_url: 'https://api-inference.modelscope.cn/v1',
      model_name: 'Qwen/Qwen3-235B-A22B',
      protocol: 'openai',
    },
  },
  {
    id: 'modelscope-anthropic',
    label: '魔搭 · Anthropic 兼容',
    form: {
      name: '魔搭 Anthropic',
      provider: 'ModelScope',
      base_url: 'https://api-inference.modelscope.cn',
      model_name: 'deepseek-ai/DeepSeek-V4-Flash',
      protocol: 'anthropic',
    },
  },
  {
    id: 'deepseek',
    label: 'DeepSeek 官方',
    form: {
      name: 'DeepSeek',
      provider: 'DeepSeek',
      base_url: 'https://api.deepseek.com/v1',
      model_name: 'deepseek-chat',
      protocol: 'openai',
    },
  },
  {
    id: 'dashscope',
    label: '通义千问 · DashScope',
    form: {
      name: '通义千问',
      provider: 'Alibaba',
      base_url: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
      model_name: 'qwen-plus',
      protocol: 'openai',
    },
  },
  {
    id: 'moonshot',
    label: 'Moonshot · Kimi',
    form: {
      name: 'Moonshot Kimi',
      provider: 'Moonshot',
      base_url: 'https://api.moonshot.cn/v1',
      model_name: 'moonshot-v1-8k',
      protocol: 'openai',
    },
  },
  {
    id: 'zhipu',
    label: '智谱 · GLM-4 Flash',
    form: {
      name: '智谱 GLM',
      provider: 'Zhipu',
      base_url: 'https://open.bigmodel.cn/api/paas/v4',
      model_name: 'glm-4-flash',
      protocol: 'openai',
    },
  },
];
