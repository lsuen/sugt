export const CURRENT_VERSION = '0.2.6';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'fix', text: '修复接管后网关退出导致 Claude/Codex 一直重连；后台守护 sugt-cli serve' },
  { type: 'feat', text: '端口占用自动 Fallback（8787→9878 等）+ 20s 网关 HA 守护循环' },
  { type: 'feat', text: '控制台「任意 Agent 接入点」贴片，可编辑网关 API Key，支持局域网开关' },
  { type: 'feat', text: '7 套 Agent 接管模板（Claude/Codex/OpenCode/Aider 等），CLI env profiles 同步' },
  { type: 'feat', text: '模型对外别名 model_alias，/v1/models 暴露统一 Model ID' },
];
