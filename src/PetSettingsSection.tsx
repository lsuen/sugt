// 设置 → 桌面精灵（小酥）独立子页。
// 从外观页迁出宠物卡片，并承载休眠 / 气泡 / 视线 / 眨眼 / 免打扰等行为开关。
import React, { useEffect, useState } from 'react';
import { t } from './i18n';
import {
  PET_SIZES,
  petEnabledPatch,
  petSizePatch,
  type OverlayConfigView,
} from './pet/config';

type Props = {
  overlay?: OverlayConfigView;
  /** 默认配置（关闭精灵时回退 HUD 尺寸） */
  defaults: OverlayConfigView;
  onPatch: (patch: Partial<OverlayConfigView>) => void;
};

export function PetSettingsSection({ overlay, defaults, onPatch }: Props) {
  const petOn = Boolean(overlay?.enabled);
  // 互动：会话开关与「长期互动」合并展示，任一为真即视为开启
  const interactiveOn = Boolean(overlay?.pet_interactive || overlay?.pet_always_interactive);
  // 滑块本地草稿：松手 / 方向键再写入，避免拖动时每步打 IPC
  const [sleepSecs, setSleepSecs] = useState(
    overlay?.pet_sleep_after_secs ?? defaults.pet_sleep_after_secs,
  );
  const [bubbleMs, setBubbleMs] = useState(
    overlay?.pet_bubble_ms ?? defaults.pet_bubble_ms,
  );
  const [opacity, setOpacity] = useState(overlay?.opacity ?? defaults.opacity);
  useEffect(() => {
    setSleepSecs(overlay?.pet_sleep_after_secs ?? defaults.pet_sleep_after_secs);
    setBubbleMs(overlay?.pet_bubble_ms ?? defaults.pet_bubble_ms);
    setOpacity(overlay?.opacity ?? defaults.opacity);
  }, [overlay?.pet_sleep_after_secs, overlay?.pet_bubble_ms, overlay?.opacity, defaults]);

  const commitSleep = (value: number) => onPatch({ pet_sleep_after_secs: value });
  const commitBubble = (value: number) => onPatch({ pet_bubble_ms: value });
  const commitOpacity = (value: number) => onPatch({ opacity: value });

  const posText =
    !overlay || overlay.x < 0 || overlay.y < 0
      ? t('set.petPos')
      : t('set.petPosAt', { x: overlay.x, y: overlay.y });

  // 启用态只写 enabled：尺寸、落点、不透明度与行为开关各自独立持久化。
  const togglePet = (enabled: boolean) => onPatch(petEnabledPatch(enabled));

  const setPetSize = (size: (typeof PET_SIZES)[number]) => onPatch(petSizePatch(size));

  const setInteractive = (enabled: boolean) => {
    // 合并写入：长期互动与会话互动同开同关，避免两套开关打架。
    // 关着精灵时也只记录偏好，不再把精灵自动打开。
    onPatch({
      pet_interactive: enabled,
      pet_always_interactive: enabled,
    });
  };

  const cfg = overlay ?? defaults;

  return (
    <div className="settings-layout settings-layout--stack">
      <div className="card settings-group switches settings-group-wide">
        <h4>{t('set.petTitle')}</h4>
        <p className="hint compact">{t('set.petHint')}</p>
        <label className="switch-line">
          <span>{t('set.petEnable')}</span>
          <input
            type="checkbox"
            checked={petOn}
            onChange={(e) => togglePet(e.target.checked)}
          />
        </label>

        {/* 行为开关始终可见：关精灵时也可改，下次开启直接生效 */}
        <label className="switch-line">
          <span>{t('set.petInteractive')}</span>
          <input
            type="checkbox"
            checked={interactiveOn}
            onChange={(e) => setInteractive(e.target.checked)}
          />
        </label>
        <p className="hint compact">{t('set.petInteractiveHint')}</p>

        <p className="hint compact">{t('set.petClickThroughHint')}</p>

        <div className="switch-line">
          <span>{t('set.petSize')}</span>
          <div className="overlay-layout-switch">
            {PET_SIZES.map((size) => (
              <button
                key={size.key}
                type="button"
                className={`ghost tiny-btn${Math.round(cfg.width) === size.width ? ' active' : ''}`}
                disabled={!petOn}
                onClick={() => setPetSize(size)}
              >
                {t(size.label)}
              </button>
            ))}
          </div>
        </div>

        <div className="switch-line">
          <span>{t('set.petOpacity', { p: Math.round(opacity * 100) })}</span>
          <input
            type="range"
            min={0.15}
            max={1}
            step={0.05}
            value={opacity}
            disabled={!petOn}
            onChange={(e) => setOpacity(Number(e.currentTarget.value))}
            onMouseUp={(e) => commitOpacity(Number(e.currentTarget.value))}
            onTouchEnd={(e) => commitOpacity(Number(e.currentTarget.value))}
            onKeyUp={(e) => {
              if (e.key.startsWith('Arrow')) commitOpacity(Number(e.currentTarget.value));
            }}
          />
        </div>
        <p className="hint compact">{t('set.petOpacityHint')}</p>

        <label className="switch-line">
          <span>{t('set.petSnap')}</span>
          <input
            type="checkbox"
            checked={cfg.pet_snap !== false}
            onChange={(e) => onPatch({ pet_snap: e.target.checked })}
          />
        </label>
        <p className="hint compact">{t('set.petSnapHint')}</p>

        <div className="switch-line">
          <span>{t('set.petSleep', { s: sleepSecs })}</span>
          <input
            type="range"
            min={60}
            max={1800}
            step={30}
            value={sleepSecs}
            onChange={(e) => setSleepSecs(Number(e.currentTarget.value))}
            onMouseUp={(e) => commitSleep(Number(e.currentTarget.value))}
            onTouchEnd={(e) => commitSleep(Number(e.currentTarget.value))}
            onKeyUp={(e) => {
              if (e.key.startsWith('Arrow')) commitSleep(Number(e.currentTarget.value));
            }}
          />
        </div>
        <p className="hint compact">{t('set.petSleepHint')}</p>

        <div className="switch-line">
          <span>{t('set.petBubble', { ms: bubbleMs })}</span>
          <input
            type="range"
            min={1500}
            max={12000}
            step={500}
            value={bubbleMs}
            onChange={(e) => setBubbleMs(Number(e.currentTarget.value))}
            onMouseUp={(e) => commitBubble(Number(e.currentTarget.value))}
            onTouchEnd={(e) => commitBubble(Number(e.currentTarget.value))}
            onKeyUp={(e) => {
              if (e.key.startsWith('Arrow')) commitBubble(Number(e.currentTarget.value));
            }}
          />
        </div>
        <p className="hint compact">{t('set.petBubbleHint')}</p>

        <label className="switch-line">
          <span>{t('set.petGaze')}</span>
          <input
            type="checkbox"
            checked={cfg.pet_gaze !== false}
            onChange={(e) => onPatch({ pet_gaze: e.target.checked })}
          />
        </label>
        <p className="hint compact">{t('set.petGazeHint')}</p>

        <label className="switch-line">
          <span>{t('set.petBlink')}</span>
          <input
            type="checkbox"
            checked={cfg.pet_blink !== false}
            onChange={(e) => onPatch({ pet_blink: e.target.checked })}
          />
        </label>
        <p className="hint compact">{t('set.petBlinkHint')}</p>

        <label className="switch-line">
          <span>{t('set.petQuiet')}</span>
          <input
            type="checkbox"
            checked={Boolean(cfg.pet_quiet)}
            onChange={(e) => onPatch({ pet_quiet: e.target.checked })}
          />
        </label>
        <p className="hint compact">{t('set.petQuietHint')}</p>

        <label className="switch-line">
          <span>{t('set.petRoam')}</span>
          <input
            type="checkbox"
            checked={cfg.pet_roam !== false}
            onChange={(e) => onPatch({ pet_roam: e.target.checked })}
          />
        </label>
        <p className="hint compact">{t('set.petRoamHint')}</p>

        <div className="settings-actions-row">
          <button
            type="button"
            className="ghost tiny-btn"
            disabled={!petOn}
            onClick={() => onPatch({ x: -1, y: -1, edit: false })}
          >
            {t('set.petReset')}
          </button>
          <span className="hint compact">{posText}</span>
        </div>
      </div>
    </div>
  );
}
