import { loadCustomBg, persistCustomBg, type CustomBgState } from './themeStorage';

const STYLE_ATTR = 'data-sugt-custom-bg';
export const MAX_BG_BYTES = 1.6 * 1024 * 1024;
export const DEFAULT_CUSTOM_BG_OPACITY = 0.4;

function clearStyle(): void {
  document.querySelectorAll(`style[${STYLE_ATTR}]`).forEach((n) => n.remove());
}

/** Apply custom bg on `.app-shell` so it overrides theme `--bg-image` for the shell only. */
export function applyCustomBg(state: CustomBgState | null): void {
  clearStyle();
  if (!state?.dataUrl) return;
  const opacity = Math.min(1, Math.max(0, state.opacity));
  const el = document.createElement('style');
  el.setAttribute(STYLE_ATTR, '1');
  el.textContent = `
.app-shell {
  --bg-image: url(${JSON.stringify(state.dataUrl)});
  --bg-image-opacity: ${opacity};
}
`.trim();
  document.head.appendChild(el);
}

export function bootstrapCustomBg(): void {
  applyCustomBg(loadCustomBg());
}

export async function fileToDataUrl(file: File): Promise<string> {
  if (!file.type.startsWith('image/')) {
    throw new Error('请选择图片文件（jpg / png / webp）');
  }
  if (file.size > MAX_BG_BYTES) {
    throw new Error(`背景图过大（${(file.size / 1024 / 1024).toFixed(1)}MB），请压缩到约 1.5MB 以内`);
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === 'string') resolve(reader.result);
      else reject(new Error('无法读取图片'));
    };
    reader.onerror = () => reject(new Error('无法读取图片'));
    reader.readAsDataURL(file);
  });
}

export function saveCustomBg(state: CustomBgState): void {
  persistCustomBg(state);
  applyCustomBg(state);
}

export function clearCustomBg(): void {
  persistCustomBg(null);
  applyCustomBg(null);
}
