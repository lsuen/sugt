export const CURRENT_VERSION = '0.1.4';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'fix', text: '统一深色主题下拉样式，修复白底灰字；调用来源改为弹窗展示不撑破布局' },
  { type: 'feat', text: '服务商预设合并为单条（魔搭等），切换协议自动匹配官方 Base URL' },
  { type: 'feat', text: 'Model Name 支持手动填写与「获取模型」拉取列表；新增火山、硅基、MiniMax 等预设' },
  { type: 'feat', text: '控制台统计今日 Token（入/出），来自响应 usage 字段累计' },
];
