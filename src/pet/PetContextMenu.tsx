// 桌面精灵右键快捷菜单：紧凑动作面板，覆盖立绘浮出。
// 菜单打开期间由 usePetGaze 保持窗口可点击，空白区仍穿透。
import { t } from '../i18n';
import { PET_SIZES, type OverlayConfigView } from './config';

type Props = {
  cfg: OverlayConfigView;
  onSettings: () => void;
  onToggleQuiet: () => void;
  onToggleRoam: () => void;
  onSize: (size: (typeof PET_SIZES)[number]) => void;
  onDock: () => void;
  onHide: () => void;
  onClose: () => void;
};

export function PetContextMenu({
  cfg,
  onSettings,
  onToggleQuiet,
  onToggleRoam,
  onSize,
  onDock,
  onHide,
  onClose,
}: Props) {
  const act = (fn: () => void) => () => {
    fn();
    onClose();
  };

  return (
    <div className="pet-menu" role="menu">
      <button type="button" className="pet-menu-item" onClick={act(onSettings)}>
        {t('pet.menu.settings')}
      </button>
      <button type="button" className="pet-menu-item" onClick={act(onToggleQuiet)}>
        {cfg.pet_quiet ? t('pet.menu.unquiet') : t('pet.menu.quiet')}
      </button>
      <button type="button" className="pet-menu-item" onClick={act(onToggleRoam)}>
        {cfg.pet_roam !== false ? t('pet.menu.unroam') : t('pet.menu.roam')}
      </button>
      <div className="pet-menu-size">
        <span className="pet-menu-label">{t('pet.menu.size')}</span>
        <div className="pet-menu-size-btns">
          {PET_SIZES.map((size) => (
            <button
              key={size.key}
              type="button"
              className={`pet-menu-item pet-menu-size-btn${Math.round(cfg.width) === size.width ? ' is-active' : ''}`}
              onClick={act(() => onSize(size))}
            >
              {t(size.label)}
            </button>
          ))}
        </div>
      </div>
      <button type="button" className="pet-menu-item" onClick={act(onDock)}>
        {t('pet.menu.dock')}
      </button>
      <button type="button" className="pet-menu-item pet-menu-item--danger" onClick={act(onHide)}>
        {t('pet.menu.hide')}
      </button>
    </div>
  );
}
