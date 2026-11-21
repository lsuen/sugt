# scripts/ — 定版脚本副本与统一入口

> **维护人**：孙测测　|　**日期**：2026-11-10（v1.3）／2026-11-11 增补（v1.4）
> **口径声明（v1.4）**：本页 `1000 帧` / `20 动作` / `180×297` / `41 张` / `76 项` / `124/124` 等均为 v1.3.x（20 动作时期）历史定版或验收数，按当时结论保留、不回填改数字；契约门禁沿革 76/76 → 78/78 → 83/83 → **v1.4 84/84**（新增 C48 高度主导不变式），落包逐帧沿革 83/83 → 124/124 → **v1.4 148/148**。v1.4 现行实测为 `240×297` / 24 动作 / 1200 帧 / 49 张 / **5669500 B = 5.670 MB = 5.41 MiB**（预算 ≤8 MB）；`petctl.py doctor` 全部通过（12 个定版脚本副本 sha256 一致、配置快照一致、49 张、5.670 MB）。上下文锚点 `docs/小精灵维护/v1.4执行契约.md`；补边脚本 `docs/孙测测/scripts/pet_v14_pad_canvas.py`。

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
| `check_pet_overlay.py` | SCR-006 | 84 项全链路契约门禁（沿革 v1.3 76/76 → v1.3.2 78/78 → v1.3.3 83/83 → **v1.4 84/84 实测通过**；v1.4 新增 C48 高度主导不变式） |
| `pet_v14_pad_canvas.py` | SCR-022 | **v1.4 扩边**：`180×297 → 240×297` 逐帧纯 paste（左右各 +30px 全透明），`--sample` 量测、`--write` 落盘 |
| `pet_v14_finalize_assets.py` | SCR-023 | **v1.4 落包桥接**：`assess` 挑版 / `build` 锁定 `h_ref=176.04` 归一化 + 同高量测 / `apply` 落包，取代单动作跑 `--phase frames`；v1.5 起归一化参数全部可 CLI 覆盖 |
| `pet_metrics.py` | SCR-024 | **v1.5 人物尺度量测**：只读逐帧量身高 / 头顶与头部下沿 / 头长 / 头身比 / 宽度 / 脚底线 / 中心 / 四边留白；`--plan` 推算裁切外扩与缩放平移传参；参数缺省读 `config/normalize-baseline.json`，CLI 可逐项覆盖，不写素材、不调 ComfyUI |

逐条参数、示例、留痕文件名见上层《脚本说明手册.md》。

## 副本规则（重要）

- **编辑源是 `docs/孙测测/scripts/`**，本目录是**只读副本**。改脚本请去编辑源改，然后：
  ```bash
  /usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py sync
  ```
- 两个目录层级同为 `docs/<目录>/scripts/`，所以脚本内按 `__file__` 推导的**项目根、留痕目录、master 路径全部一致**，本目录副本可以**直接运行**，不用改代码。
- `petctl.py doctor` 会逐项比对 `FINAL_SCRIPTS` 里 **13 个定版副本**与原件的 sha256，漂移即判红；也会检查本目录 `config/manifest.json`、`assets/su-cel-good-fixed.png` 是否与生产输入一致。
- **只有这 13 个（外加统一入口 `petctl.py` 本身）是定版**。试片失败、v0.7 洋红底链路、Playwright 旧冒烟等 10 个脚本故意不收录，清单见《脚本说明手册》第四节。

## 三条最常用命令

```bash
P=/usr/local/bin/python3.13
cd <项目根>

$P docs/小精灵维护/scripts/petctl.py doctor     # 环境 + 副本 + 素材自检
$P docs/小精灵维护/scripts/petctl.py all        # 一键回归（含 npm test / tsc --noEmit / build / 84 项门禁；v1.4 24 动作实测 84/84、落包逐帧 148/148 全绿）
$P docs/小精灵维护/scripts/petctl.py snapshot   # 把 49 张（v1.4 现行实测；v1.3.3 历史为 41 张）进包 WebP 归档到 ../assets/final/（执行后才生成的条件目录）
```

## 环境

- Python 必须是 `/usr/local/bin/python3.13`（素材链依赖 Pillow）。
- 生产重生成需要本地 ComfyUI `192.168.31.84:8188` 与 `SUGT_PET_WORK`；`doctor` 不探测服务。
- 所有脚本自动往 `docs/孙测测/traces/` 写留痕；试跑可加 `--out /tmp/<name>.md`，不污染仓库。
- 退出码：`0` 通过 / `1` 不通过 / `2` 读取、解析或依赖异常。
