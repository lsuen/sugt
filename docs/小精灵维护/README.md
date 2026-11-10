# 小精灵更新维护手册 · 总入口

> **适用版本**：SUGT 桌面精灵 v1.3.3（20 动作 × 50 帧 = 1000 张独立位图 / `180×297` / 56 色）
> **本轮定稿**：v1.3.2 用**原始 H3 视频离线重建**，不重新生成、不裁任何一像素人物；抠图结果缓存在 `SUGT_PET_WORK/cutouts/`，调人物尺寸只需 `--phase frames`（零 ComfyUI）。**v1.3.3 仅 `wake` 一段用 `640×768` 本地 ComfyUI 重生成**（修复源画布裁掉手掌），其余 19 段仍是 512×768 旧视频、按用户要求「先记着、以后再优化」暂不动；同时把自主漂移改为 `lifted → landing`。详见《更新维护手册》4.3 / 4.3.1 / 7.2-B2。
> **建立日期**：2026-11-10　|　**维护人**：孙测测　|　**唯一进包路径**：`src/assets/pet/`
> **目的**：拿到这个目录，不用翻历史对话、不用猜哪份脚本能用，就能改动作、改尺寸、重生成素材、验收放行。

---

## 一、5 分钟上手

```bash
# 0) 环境自检（必做第一步：副本是否与定版原件一致、素材是否齐全）
/usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py doctor

# 1) 只改了配置 / 动作表 / CSS，跑一键回归
/usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py all

# 2) 只想看看 20 个动作长什么样
#    打开 assets/动作接触图.png，或 docs/design/pet-v07-preview.html（可点选演示）
```

**Python 用 `/usr/local/bin/python3.13`**：所有素材链脚本依赖 Pillow，系统默认 `python3` 没有。

**脚本有两份，但只有一份能改**：`scripts/` 是定版副本（可直接运行），编辑源是 `docs/孙测测/scripts/`。
改脚本去编辑源改，然后 `petctl.py sync` 单向同步回本目录；`doctor` 会检查两者 sha256 是否一致。

---

## 二、目录地图

```
docs/小精灵维护/
├── README.md                  ← 本文件：总入口 / 5 分钟上手 / 三条常用流程
├── 更新维护手册.md             ← 【主手册】文件地图、配置默认值、尺寸与框、增删改查、生成流程、坑
├── 脚本说明手册.md             ← 10 个定版脚本逐条说明 + 调用顺序 + 废弃脚本清单
├── 动作表.md                  ← 20 动作完整注册表 + 优先级分层 + 贴边风险 + 扩展模板
├── scripts/                   ← 定版脚本副本（10 个）+ 统一入口
│   ├── README.md              ← 副本规则 / 唯一编辑源 / 同步方式
│   ├── petctl.py              ← SCR-021 统一入口（doctor / all / gen / produce / sync …）
│   └── *.py                   ← 10 个定版脚本，可直接运行
├── config/
│   └── manifest.json          ← 动作表唯一真相源「快照」（与 src/assets/pet/manifest.json 一致）
└── assets/
    ├── 动作接触图.png          ← 20 动作静态海报拼图，一眼查动作
    ├── su-cel-good-fixed.png  ← master 基准图（用户手抠 good.png 经 SCR-017 去溢色），重生成的输入
    ├── 素材清单.md             ← 41 张 WebP 的名称 / 尺寸 / 帧数 / 字节 / sha256
    └── final/                 ← 可选：执行 petctl.py snapshot 后才有，41 张完整快照
```

**没放进来的东西**（刻意为之）：

| 未收录 | 原因 |
| --- | --- |
| 41 张成品 WebP 的默认副本 | 避免同一份素材在仓库里出现两份真相源；需要时 `petctl.py snapshot` 一键拉快照 |
| `docs/design/pet-v07-preview.html`（10 MB，内联素材） | 由 `build_pet_preview.py` 生成，改了要重跑，不手改 |
| 试片 / 早期 v0.7 脚本 11 个 | 效果不好或已被 v1.3 链路取代，清单见《脚本说明手册》第四节 |
| `docs/孙测测/traces/` 留痕 | 仍是唯一留痕目录，脚本自动写入，不在本目录重复 |

---

## 三、三条常用流程（照抄即可）

### 流程 A：只改参数（尺寸、默认值、节奏、优先级），不动素材

1. 改 `src/assets/pet/manifest.json` 或 `src/pet/config.ts` / `src-tauri/src/model.rs`。
2. `petctl.py all`（生成常量 → 预览页 → 自检 → 1000 帧 → 单测 → 构建 → 76 项契约门禁）。
3. 目视 `docs/design/pet-v07-preview.html`，实窗确认拖拽三段与右键菜单。

### 流程 B：新增 / 替换一个动作（不动其余 19 个）

1. 在 `manifest.json` 的 `poses[]` 追加一条（模板见《动作表》第五节），放好两张专属 WebP。
2. `petctl.py produce --phase keys --action <新动作>` → 单动作关键姿态。
3. `petctl.py produce --phase interpolate --action <新动作>` → H3 补间。
4. `petctl.py series --reference <工作目录>/su-master.png --video <工作目录>/videos/<新动作>.webp --out /tmp/xxx` → 序列级验收。
5. `petctl.py produce --phase frames` → 全量重新归一化（**必须全量**，统一坐标系不能只算一个动作）。
6. `petctl.py reencode --source <工作目录>/source` → 整套 41 张重编码。
7. `petctl.py all` → 全门禁。

> 帧预算是硬契约：新增 1 个动作 = 总帧数变 1050，必须同步改 `actionRegistry.frameBudget.total`，否则 C36 判红。

### 流程 C：换造型 / 换 master，全量重生成 1000 帧

1. 新定稿透明 PNG → `petctl.py cutout --image <新图> --mode chroma --out /tmp/cutout`（可进包抠图门禁）。
2. `petctl.py asset --image <新图> --source <原图> --fix --out /tmp/asset` → **修图**：去 1~3px 溢色绿边，拿修补版 PNG。
3. 把修补版放到 `docs/孙测测/traces/assets/su-cel-good-fixed.png`（SCR-019 硬编码读这个路径），同步更新本目录 `assets/su-cel-good-fixed.png`。
4. 设生产环境变量后跑 `petctl.py produce --phase all`：
   ```bash
   export SUGT_PET_WORK=/tmp/sugt-pet-actions-v13
   export SUGT_PET_CANVAS=640x768      # v1.3.3 起下限，512 宽会裁掉手指 / 发梢
   export SUGT_PET_H3_STEPS=4          # 脚本默认 8；定版实测用 4，不显式设会变慢且画质不同
   export SUGT_PET_EDIT_STEPS=4
   ```
5. 全量 20 段序列验收 → `--phase frames` 重新归一化 → `reencode --source …/source` → `petctl.py all`。
6. 目视 `assets/动作接触图.png`（需重生成）与预览页，确认 20 动作语义、正立、全身到鞋。

---

## 四、先读哪一份

| 你的问题 | 看哪份 |
| --- | --- |
| 「背景框 / 窗口大小在哪调」 | 《更新维护手册》第四节「尺寸、框与显示不全」 |
| 「有的动作显示框外显示不全」 | 《更新维护手册》第四节 + 《动作表》第三节贴边实测 |
| 「加 / 删 / 改一个动作」 | 《更新维护手册》第六节 + 《动作表》第五节模板 |
| 「这个脚本干嘛的、怎么跑」 | 《脚本说明手册》 |
| 「现在有哪些动作、谁优先」 | 《动作表》第一、二节 |
| 「重生成素材的完整命令」 | 《更新维护手册》第七节 / 本文件流程 C |
| 「之前踩过什么坑」 | 《更新维护手册》第九节（24 条，带文件行号与实测数据） |
| 「默认值是多少」 | 《更新维护手册》第三节 |

---

## 五、红线速记（违反必出事）

1. **唯一真相源是 `manifest.json`**：动作、帧数、尺寸、色数、节奏都从它生成，不要在 CSS / TS / 脚本里写第二份。
2. **每个动作一张专属 poster + 一张专属 loop**：禁止跨语义复用，禁止拖拽复用走路姿。
3. **每条 `.pet-loop[data-loop]` / `.pet-poster[data-frame]` 只能有一张 `background-image`**：CSS 多值是叠加不是 fallback，透明 WebP 会露出下层立绘成双影。
4. **整套替换**：换造型必须 41 张一起换，只换静态海报会让同一 loop 里混进两套形象。
5. **拖拽是三段不是布尔**：按下 `lifted`（被提起来）→ 松手 `landing`（落地缓冲）→ `idle`；纯单击不播落地。
6. **云端 ImageGen / VideoGen 禁止进包**：无 alpha、带水印、跨图漂移；素材只走本地 ComfyUI。
7. **`--emit-only` 不是验收证据**：它跳过素材核对，只能用于生产中途生成常量。
8. **Git 操作前先报告并确认**：本项目约定，别顺手 commit。

---

## 六、放行清单（上线前逐条打勾）

- [ ] `petctl.py doctor` 全绿（副本无漂移、master 一致、41 张 / ≤8 MB）
- [ ] `petctl.py frames` **124/124**（1000 帧独立位图、四边留白 ≥6px、20 条 hitBox 与 alpha 并集一致、blink 与 sleep poster 字节不同）
- [ ] `petctl.py check` 2/2（尺寸 / 帧数 / 时长 / alpha 与 manifest 一致）
- [ ] `petctl.py all`：gen 5/5、预览页、`npm test` 三套件 + `tsc --noEmit`、`npm run build`、**契约门禁 83/83**（v1.3.3 新增 C46 系列漫游语义 / C47 生产四边门禁）
- [ ] 目视：`v1.3.3最终素材接触图.png` 20 格大小一致、`v1.3.3极限帧验收图.png` 的 wake 双手完整、预览页 20 动作 + 拖拽三段
- [ ] 实窗口人工验收：按住被提起 / 松手落地 / 纯单击不落地 / 右键菜单 / 默认不穿透 / 跨屏 DPI / 动作切换不跳位 / **自主漂移像「被提起来飞过去」且落点自然落地**
- [ ] 许可口径：本地生成素材、中国大陆分发，MiniMax H3 Community License 地域限制已登记在 `ASSETS.md` 第六节
