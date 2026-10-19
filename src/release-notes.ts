export const CURRENT_VERSION = '1.1.3';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '新增「全局规则」：规则正文、文档习惯、Webhook 各自开关' },
  { type: 'feat', text: '新配置预置通用规则（默认不注入），并默认开启 AGENTS.md 与 docs/MAP.md' },
  { type: 'feat', text: '文档习惯可写「作用与说明」；Webhook 只在本地代发，不进入模型上下文' },
];
