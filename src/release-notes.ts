export const CURRENT_VERSION = '0.1.5';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'fix', text: '按官方文档校正各服务商 OpenAI/Anthropic Base URL，火山等支持 Anthropic 不再禁用' },
  { type: 'feat', text: '新增 docs/provider-vendors.md 与后端 provider_catalog，模型列表适配 /v3、/v4 等路径' },
  { type: 'fix', text: 'Anthropic 协议可选且未知时不锁死；无列表接口的厂商提示手填 Model Name' },
];
