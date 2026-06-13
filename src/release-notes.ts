export const CURRENT_VERSION = '0.1.3';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '控制台增加请求统计卡片：今日/累计请求、耗时、失败率与 60 分钟迷你图' },
  { type: 'feat', text: '添加模型改为国内服务商下拉预设（魔搭、DeepSeek、通义、Kimi、智谱等）' },
  { type: 'fix', text: '移除顶部全局刷新按钮，改为自动刷新；可展开查看调用来源' },
];
