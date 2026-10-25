export const CURRENT_VERSION = '1.1.7';

export type ReleaseNote = {
  type: 'fix' | 'feat';
  text: string;
};

export const RELEASE_NOTES: ReleaseNote[] = [
  { type: 'fix', text: 'Codex：config.toml 写入 model_providers.sugt + wire_api=responses；Chat 回落保留 tools / additional_tools' },
  { type: 'fix', text: 'Codex：reasoning 与正文分流；仅推理时才兜底进 output_text；流结束日志 content/reasoning/tools' },
  { type: 'feat', text: '轻量主题皮肤（含二次元包）、主题切换修复、品牌区与 ModalPortal 叠层' },
];
