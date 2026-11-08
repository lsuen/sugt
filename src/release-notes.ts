export const CURRENT_VERSION = '1.1.9';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'feat', text: 'Linux：CI 自动打包 sugt-cli 无头网关（x64 / arm64），Release 固定资产名不含版本号' },
  { type: 'feat', text: '服务器模式：sugt-cli serve --lan 绑定局域网；非回环绑定时强制校验客户端 API Key，避免上游 Key 被蹭用' },
  { type: 'feat', text: '附带 systemd 样例（deploy/sugt-gateway.service），便于企业把 SUGT 当局域网网关服务部署' },
  { type: 'fix', text: 'Codex 工具层：补齐 Responses → Chat 回落的工具判定、SSE 生命周期与失败对象，接管后不再出现 shell / MCP / 网页工具全挂' },
  { type: 'fix', text: 'Codex 释放接管：按快照精确还原用户原有配置与登录态；兼容 1.1.7 无快照的遗留态，Windows 原子写不再先删后搬' },
  { type: 'feat', text: '设置重组：主导航收敛为 7 个页签，日志 / 关于移入设置的 6 个冻结子页签，新增只读概览页' },
  { type: 'feat', text: '客户端目录扩展到 22 个国内外 Agent，空查询即可列出完整目录；接管影响改为弹窗预览' },
  { type: 'feat', text: '技能：粘贴 GitHub 地址一步安装到本地技能，展示触发词与规则化优化建议' },
  { type: 'feat', text: 'SOUL 升级为只读编排层：给出客户端 / 技能 / 接管影响 / 设置的跳转建议，模型不可用时功能照常可用' },
  { type: 'fix', text: '窗口初始高度补偿状态栏 34px，并记住用户调整后的尺寸；接入影响弹窗与长文本溢出等界面问题一并修复' },
  { type: 'feat', text: '发布链路：Windows / macOS（arm64 + x64）自动打包，release 资产名不再带版本号，Homebrew 与下载站始终指向最新版' },
];
