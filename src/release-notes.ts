export const CURRENT_VERSION = '1.1.10';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'fix', text: '声明了工具却只回文字时，同一条请求里再要：免费通道按 3、2、1，收费模型可在添加/编辑页自选次数。出现工具调用立刻交回，次数用尽才把文字交回' },
  { type: 'fix', text: '429 和 5xx 在同一条请求里重试，预算用尽回助手提示，会话不断。对外模型名固定。没带 max_tokens 的请求填 32768' },
  { type: 'feat', text: '应用图标换成新标志，托盘、安装包和界面标志一起更换' },
  { type: 'feat', text: '桌面精灵小su：千帧表驱动动作（20 动作 × 50 帧）、拖拽起落三段、睡醒/漫游与活感层；默认开启、中号窗口' },
  { type: 'feat', text: '精灵设置独立子页：尺寸 / 休眠 / 气泡 / 视线 / 眨眼 / 免打扰 / 走动；右键菜单快捷操作' },
  { type: 'feat', text: 'Linux：CI 自动打包 sugt-cli 无头网关（x64 / arm64），Release 固定资产名不含版本号' },
  { type: 'feat', text: '服务器模式：sugt-cli serve --lan 绑定局域网；非回环绑定时强制校验客户端 API Key' },
  { type: 'feat', text: '附带 systemd 样例（deploy/sugt-gateway.service），便于把 SUGT 当局域网网关服务部署' },
  { type: 'fix', text: 'Codex 工具层：Responses → Chat 回落补齐工具判定、SSE 生命周期与失败对象；截断 tool_call 不再静默丢参' },
  { type: 'fix', text: 'Codex 释放接管：按快照精确还原配置与登录态；兼容 1.1.7 无快照遗留态' },
  { type: 'feat', text: '技能本地页：添加折叠、勾选后挂载条、行信息降噪；推荐库刷新单仓失败不阻断其它仓' },
  { type: 'feat', text: '客户端展示名对齐：Codex ChatGPT；控制台去掉开发向「最近命中」文案' },
  { type: 'fix', text: '主窗口默认 1040×730，并继续记住用户调整后的尺寸' },
  { type: 'feat', text: '设置重组 / 22 客户端目录 / GitHub 技能安装 / SOUL 只读编排 / 发布链路固定资产名（延续）' },
];
