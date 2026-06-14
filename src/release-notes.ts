export const CURRENT_VERSION = '0.2.5';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'fix', text: '服务商 Base URL 按官方文档校正，火山等 Anthropic 协议不再禁用' },
  { type: 'feat', text: '新增 docs/provider-vendors.md，模型列表适配火山 /v3、智谱 /v4 等' },
  { type: 'feat', text: '同步网关 0.1.5：MiniMax/硅基/DeepSeek 等 Anthropic 地址与获取模型优化' },
  { type: 'feat', text: '商店：GitHub 代理与环境 Tab 等 0.2.x 能力保留' },
];
