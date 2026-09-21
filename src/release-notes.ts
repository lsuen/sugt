export const CURRENT_VERSION = '1.1.2';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: '新增实验·免费通道（替代原 OpenCode Zen），内置只读免费上游并可探测可用性' },
  { type: 'feat', text: '请求统计：调用来源展示 Token；新增调用通道汇总（请求数 / Token）' },
  { type: 'fix', text: '修复非 2xx 上游响应未计入失败的问题' },
  { type: 'feat', text: '重做流量悬浮窗：展示今日请求、成功率、Token、来源与通道（与请求统计同源）' },
  { type: 'feat', text: '接入点支持「直接体验」快捷对话与代码范例' },
];
