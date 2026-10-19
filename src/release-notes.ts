export const CURRENT_VERSION = '1.1.3';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '新增「全局规则」：跨 agent 软约束注入，可开关并持久化' },
  { type: 'feat', text: '目录地图习惯提醒 + 钉钉 webhook 本地代发（/v1/_sugt/notify，密钥不进模型）' },
];
