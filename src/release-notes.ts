export const CURRENT_VERSION = '0.2.4';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'fix', text: '统一深色下拉样式；调用来源改弹窗展示' },
  { type: 'feat', text: '同步网关：服务商预设与协议切换 Base URL、获取模型列表、Token 统计' },
  { type: 'feat', text: '新增火山、硅基流动等国内服务商预设' },
  { type: 'feat', text: '发现技能 GitHub 代理与环境 Tab 增强' },
];
