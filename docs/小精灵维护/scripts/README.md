# scripts/ — 定版脚本副本与统一入口

> **维护人**：孙测测　|　**日期**：2026-11-10

## 这里有什么

| 文件 | 编号 | 角色 |
| --- | --- | --- |
| `petctl.py` | SCR-021 | **统一入口**：`doctor` / `all` / `gen` / `produce` / `sync` … |
| `comfyui_curl_http.py` | SCR-012 | curl 传输适配器（被 SCR-019 import） |
| `check_pet_cutout_asset.py` | SCR-017 | **修图**：已抠 PNG 门禁 + `--fix` 去溢色绿边 |
| `check_pet_keyvisual_cutout.py` | SCR-016 | 定稿图「可进包抠图」门禁 |
| `comfyui_pet_action_series.py` | SCR-019 | **ComfyUI 生图**：1000 帧批量生产线 |
| `check_pet_pose_series.py` | SCR-018 | 生产视频 / 姿态序列级验收 |
| `pet_reencode_assets.py` | SCR-011 | 整套 56 色重编码落包 + `--check` |
| `check_pet_final_frames.py` | SCR-020 | 落包 1000 帧逐帧验收 |
| `gen_pet_from_manifest.py` | SCR-010 | manifest → CSS / TS 生成器 |
| `build_pet_preview.py` | SCR-005 | 离线互动预览页 |
| `check_pet_overlay.py` | SCR-006 | 76 项全链路契约门禁 |

逐条参数、示例、留痕文件名见上层《脚本说明手册.md》。

## 副本规则（重要）

- **编辑源是 `docs/孙测测/scripts/`**，本目录是**只读副本**。改脚本请去编辑源改，然后：
  ```bash
  /usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py sync
  ```
- 两个目录层级同为 `docs/<目录>/scripts/`，所以脚本内按 `__file__` 推导的**项目根、留痕目录、master 路径全部一致**，本目录副本可以**直接运行**，不用改代码。
- `petctl.py doctor` 会逐项比对 10 个副本与原件的 sha256，漂移即判红；也会检查本目录 `config/manifest.json`、`assets/su-cel-good-fixed.png` 是否与生产输入一致。
- **只有这 10 个是定版**。试片失败、v0.7 洋红底链路、Playwright 旧冒烟等 10 个脚本故意不收录，清单见《脚本说明手册》第四节。

## 三条最常用命令

```bash
P=/usr/local/bin/python3.13
cd <项目根>

$P docs/小精灵维护/scripts/petctl.py doctor     # 环境 + 副本 + 素材自检
$P docs/小精灵维护/scripts/petctl.py all        # 一键回归（含 npm test / build / 76 项门禁）
$P docs/小精灵维护/scripts/petctl.py snapshot   # 把 41 张进包 WebP 归档到 ../assets/final/
```

## 环境

- Python 必须是 `/usr/local/bin/python3.13`（素材链依赖 Pillow）。
- 生产重生成需要本地 ComfyUI `192.168.31.84:8188` 与 `SUGT_PET_WORK`；`doctor` 不探测服务。
- 所有脚本自动往 `docs/孙测测/traces/` 写留痕；试跑可加 `--out /tmp/<name>.md`，不污染仓库。
- 退出码：`0` 通过 / `1` 不通过 / `2` 读取、解析或依赖异常。
