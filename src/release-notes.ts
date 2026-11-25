export const CURRENT_VERSION = '1.2.4';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'fix', text: 'Windows 首启：端口探测并行 + 250ms 超时，10s 卡死压到 0.3s；重复启动聚焦已有窗口' },
  { type: 'fix', text: '修复模型列表加载卡死并渐进呈现；长代码/命令预览限高，不再撑破界面' },
  { type: 'fix', text: 'Windows 退出：置 QUITTING 并加 6s 硬超时；日志抽屉改受控轮询，避免 WebView2 空转' },
  { type: 'fix', text: '精灵右键菜单命令改为 async，解决 Windows 上右键无反应；首屏面板按路由分片，入口包减 62%' },
  { type: 'fix', text: 'Windows：setx/扫盘移出 async，避免首启卡死、接入列表转圈、关不掉；退出硬超时' },
  { type: 'fix', text: 'Windows：精灵右键菜单改为已存在则聚焦，并防抖连发事件，避免刚开即关' },
  { type: 'feat', text: '实验免费通道：分级探测缓存、仅对话能力标记、单路由重测；控制台指标紧凑展示' },
  { type: 'feat', text: '模型列表：能力徽章、截断轮播、一键禁用不可用模型' },
  { type: 'feat', text: '接入默认加入 WorkBuddy / DSH；接管写入可用 model id 与网关 API Key，偏离可预警并修复' },
  { type: 'feat', text: 'SOUL：轻任务提示、选图、伪 tool_call 一键改写 bash；Windows 精灵菜单开关态可追踪' },
  { type: 'feat', text: 'Key 池 / 接入密钥达配额时 toast 告警（仍不拦截请求）' },
  { type: 'fix', text: '免费通道欢迎条文案缩短；关窗托盘策略可配置' },
{ type: 'feat', text: '配置预设：内置稳定工作 / 省钱省流 / 极速响应三档一键切换，应用前预览字段改动；支持自定义预设的保存、覆盖、导入导出与删除' },
  { type: 'feat', text: '连通性测试改为真实 LLM 探测：返回内容、延迟与 t/s，网络通但模型失效不再误报成功' },
  { type: 'feat', text: '模型列表：按服务商批量获取、并发测试、启用/禁用，禁用即从控制台隐藏' },
  { type: 'feat', text: '策略盒子：优先级 / 权重 / 轮换 / 速度阈值四种路由，统一 model id 对外暴露' },
  { type: 'feat', text: 'API Key 池：多 key 轮换与最少使用策略，按 key 统计请求数 / Token，达配额只告警不拦截' },
  { type: 'feat', text: '请求改写规则：作用域（默认 / Codex / 客户端 / 全局）与 max_tokens、工具重试可配置' },
  { type: 'feat', text: '接入信息卡：本机 IP、baseurl 与 model id 一键复制；日志按天滚动与缓存清理入口' },
  { type: 'feat', text: '导航收口为总览 / 网关 / 规则 / 接入 / 设置五组横向子页，设置页交互保持一致' },
  { type: 'feat', text: '侧栏：总览改名控制台；SOUL / 技能提为一级主 Tab（门面在分割线之上）' },
  { type: 'fix', text: '网关首选端口可在设置里改；回退绑定时不再写回覆盖首选口，控制台提示首选/实际双口' },
  { type: 'feat', text: '设置高级页可编辑备选端口列表与上游重试预算' },
  { type: 'feat', text: '实验·免费通道：模型行独立设置弹窗（免责声明、获取并测试、闲时测速与按 t/s 轮换）' },
  { type: 'fix', text: '免费通道「获取并测试」从设置高级迁出，避免与进程级设置混在一起' },
  { type: 'fix', text: 'Codex 工具层：Responses → Chat 回落补齐工具判定、SSE 生命周期与失败对象；截断 tool_call 不再静默丢参' },
  { type: 'fix', text: 'Codex Chat 桥：历史 tool 参数规范化、免费 400 换路、熔断仅密钥终态；非 Codex 透传不再擅自填 max_tokens' },
  { type: 'fix', text: 'Codex 释放接管：按快照精确还原配置与登录态；兼容 1.1.7 无快照遗留态' },
  { type: 'feat', text: '桌面精灵小su：表驱动动作、拖拽起落三段、睡醒/漫游与活感层；默认开启、中号窗口' },
  { type: 'feat', text: 'Linux：CI 自动打包 sugt-cli 无头网关（x64 / arm64），Release 固定资产名不含版本号' },
];
