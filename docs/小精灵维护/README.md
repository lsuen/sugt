# 小精灵更新维护手册 · 总入口

> **v1.5 现行口径（2026-11-12 实测）**：25 动作 × 50 帧 = **1250 帧**、`240×297`、56 色 palette-lossless、**51 张 / 5898798 B = 5.899 MB = 5.63 MiB**（预算 ≤8 MB）；逐帧 **154/154**、静态契约 **84/84**、生成器 5/5、编码 2/2、预览页 4/4、`npm test` 295 条、Rust 275 passed / 1 ignored、dist **51 张 / 5760.5 KB**；新增闲置动作 `yawn`（本地 ComfyUI 生成，锁定 h_ref 归一化落包；关键帧与视频首/中/末帧目视通过）。
> 下文出现的 `24 动作 / 1200 帧 / 49 张 / 5669500 B / 148/148 / 5536.62 KB` 均为 **v1.4 历史值**，按沿革保留、不回填改数字；现行操作以本段、《动作表》第一节与 `docs/小精灵维护/README.md` 流程 B 为准。

> **适用版本**：SUGT 桌面精灵 **v1.4（2026-11-11 落包实测：24 动作 × 50 帧 = 1200 张独立位图 / `240×297` / 56 色 palette-lossless / 49 张 / 5669500 B = 5.670 MB = 5.41 MiB，预算 ≤8 MB；逐张字节 / sha256 见《素材清单》文末 v1.4 章节）**；上一稳定版 v1.3.3 为**历史**（20 动作 × 50 帧 = 1000 张 / `180×297` / 56 色，见《更新维护手册》与 `src/assets/pet/ASSETS.md` §十一）
> **本轮定稿**：v1.3.2 用**原始 H3 视频离线重建**，不重新生成、不裁任何一像素人物；抠图结果缓存在 `SUGT_PET_WORK/cutouts/`，调人物尺寸只需 `--phase frames`（零 ComfyUI）。**v1.3.3 仅 `wake` 一段用 `640×768` 本地 ComfyUI 重生成**（修复源画布裁掉手掌），其余 19 段仍是 512×768 旧视频、按用户要求「先记着、以后再优化」暂不动；同时把自主漂移改为 `lifted → landing`。详见《更新维护手册》4.3 / 4.3.1 / 7.2-B2。
> **建立日期**：2026-11-10　|　**维护人**：孙测测　|　**唯一进包路径**：`src/assets/pet/`
> **目的**：拿到这个目录，不用翻历史对话、不用猜哪份脚本能用，就能改动作、改尺寸、重生成素材、验收放行。
>
> **维护入口索引（v1.4）**：上下文锚点 `docs/小精灵维护/v1.4执行契约.md`（防丢上下文，接手先读，主代理维护）；本轮补边脚本 `docs/孙测测/scripts/pet_v14_pad_canvas.py`。
> **口径总声明**：本目录各手册里 `180×297` / `1000 帧` / `20 动作` / `41 张` / 三档 `120×136 / 148×168 / 180×204` / 门禁 `76 项` `83/83` / 落包 `124/124` 等，除明确带 v1.3.x 版本号的，均为**上一稳定版 v1.3.3 历史定版或 20 动作时期验收数**，不代表 v1.4 现行口径；v1.4 现行实测为 `240×297` / 24 动作 / 1200 帧 / 49 张 / 三档 `160×136 / 198×168 / 240×204`（Rust 默认 `198.0`）/ **5669500 B = 5.670 MB = 5.41 MiB**，门禁 `check_pet_overlay.py` **84/84**、SCR-020 落包逐帧 **148/148**、`gen_pet_from_manifest.py` 写模式 **5/5**、`pet_reencode_assets.py --check` **2/2**、新动作落包编码 **8/8**、`petctl.py doctor` **全部通过**，`npm test` / `tsc --noEmit` / `npm run build`（dist 49 张 / 5536.62 KB）/ `git diff --check` 全过。

---

## 一、5 分钟上手

```bash
# 0) 环境自检（必做第一步：副本是否与定版原件一致、素材是否齐全）
/usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py doctor

# 1) 只改了配置 / 动作表 / CSS，跑一键回归
/usr/local/bin/python3.13 docs/小精灵维护/scripts/petctl.py all

# 2) 只想看看现行 24 个动作长什么样
#    打开 docs/孙测测/traces/assets/精灵v1.4-24动作接触图.png（v1.4 现行 24 格）
#    或 docs/design/pet-v07-preview.html（可点选演示）；assets/动作接触图.png 是 v1.3.3 的 20 格历史图
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
├── 脚本说明手册.md             ← 13 个定版脚本逐条说明 + 调用顺序 + 废弃脚本清单
├── 动作表.md                  ← 24 动作完整注册表（v1.4）+ 优先级分层 + 贴边风险 + 扩展模板
├── scripts/                   ← 定版脚本副本（13 个）+ 统一入口
│   ├── README.md              ← 副本规则 / 唯一编辑源 / 同步方式
│   ├── petctl.py              ← SCR-021 统一入口（doctor / all / gen / produce / sync …）
│   └── *.py                   ← 13 个定版脚本，可直接运行
├── config/
│   └── manifest.json          ← 动作表唯一真相源「快照」（与 src/assets/pet/manifest.json 一致）
└── assets/
    ├── 动作接触图.png          ← 20 动作静态海报拼图（v1.3.3 历史）；v1.4 现行 24 格见 docs/孙测测/traces/assets/精灵v1.4-24动作接触图.png
    ├── su-cel-good-fixed.png  ← master 基准图（用户手抠 good.png 经 SCR-017 去溢色），重生成的输入
    ├── 素材清单.md             ← 名称 / 尺寸 / 帧数 / 字节 / sha256（v1.3.3 历史实测 41 张整表保留；文末「v1.4 现行实测」章节已回填 49 张现行实测）
    └── final/                 ← 可选：`petctl.py snapshot` 后才生成的**条件目录**，现行 49 张完整快照（v1.3.3 历史为 41 张）
```

**没放进来的东西**（刻意为之）：

| 未收录 | 原因 |
| --- | --- |
| 49 张成品 WebP 的默认副本 | 避免同一份素材在仓库里出现两份真相源；需要时 `petctl.py snapshot` 一键拉快照 |
| `docs/design/pet-v07-preview.html`（10 MB，内联素材） | 由 `build_pet_preview.py` 生成，改了要重跑，不手改 |
| 试片 / 早期 v0.7 脚本 11 个 | 效果不好或已被 v1.3 链路取代，清单见《脚本说明手册》第四节 |
| `docs/孙测测/traces/` 留痕 | 仍是唯一留痕目录，脚本自动写入，不在本目录重复 |

---

## 三、三条常用流程（照抄即可）

### 流程 A：只改参数（尺寸、默认值、节奏、优先级），不动素材

1. 改 `src/assets/pet/manifest.json` 或 `src/pet/config.ts` / `src-tauri/src/model.rs`。
2. `petctl.py all`（生成常量 → 预览页 → 自检 → 1200 帧〔20 动作时期为 1000 帧，历史〕 → 单测 → 构建 → **84 项契约门禁（v1.4 实测 84/84）**；沿革 v1.3 初期 76/76 → v1.3.2 78/78 → v1.3.3 83/83 → v1.4 84/84（新增 C48 高度主导不变式），**勿把 76 当现行数**）。
3. 目视 `docs/design/pet-v07-preview.html`，实窗确认拖拽三段与右键菜单。

### 流程 B：新增 / 替换一个动作（不动其余 24 个）

> 本流程为 2026-11-12 新增 `yawn` 时**端到端实测跑通**的唯一路径，取代早期
> 「全量 `frames` + 整套 `reencode`」写法：单动作跑 `frames` 会重算 `h_ref`
> 导致人物忽高忽矮，`encode` 会整套覆盖进包素材。

1. **前置登记**：`manifest.json` 的 `poses[]` 追加一条、帧预算 `total` 改为 `poses × 50`、
   `posePriority` 与 `autoActions.idleVariant.poses` 同步；生产链的姿态提示词表
   （编辑源 + 维护目录副本）追加同名键；`config/manifest.json` 快照同步。
   此时素材与帧预算未跟上，门禁红灯属**预期中间态**，不要误判为回归。
2. **生成**：`produce --phase prepare` → `keys` → `interpolate`，单动作串行；
   ComfyUI 探测与上传须绕办公代理（否则 502 假阴性）；每版独立 `SUGT_PET_WORK`
   与 seed offset；**期望不跑 `--phase frames`、不跑 `--phase encode`**。
3. **目视挑版**：看关键帧 + 视频首 / 中 / 末帧，确认正立、全身到鞋、语义正确、
   四边不触边；自动化门禁不替代目视签字。
4. **归一化与落包**：`pet_v14_finalize_assets.py --actions <新动作>` 的
   `build`（锁定 h_ref、同高交叉量测）→ `apply`（只编码目标动作）；
   每次落包用独立临时 root，编码前先备份现有素材。
5. **回填与联动**：`gen_pet_from_manifest.py` 写模式回填 hitBox 与生成 CSS/TS →
   复制快照 → 重生成预览页 → 同步门禁 / 冒烟 / 文档 / 技能基线里的动作数、
   帧预算、张数与字节（期望由 manifest 推导，不写死）。
6. **全量门禁**：按 `petctl.py all` 与技能包 `scripts/pet-gate.sh --set full` 收口。

> 帧预算是硬契约：v1.5 基线 `actionRegistry.frameBudget.total = 1250`（25 × 50）；
> 再新增 1 个动作 = 1300，必须同步改该字段，否则 C36 判红。历史基线
> v1.4 为 1200（24 × 50）、v1.3 为 1000（20 × 50）。

### 流程 C：换造型 / 换 master，全量重生成 1250 帧（25 动作 × 50；v1.4 为 1200、20 动作时期 1000 帧属历史）

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
5. 全量 24 段序列验收 → `--phase frames` 重新归一化 → `reencode --source …/source` → `petctl.py all`。
6. 目视 `docs/孙测测/traces/assets/精灵v1.5-25动作接触图.png`（现行 25 格）与预览页，确认 25 动作语义、正立、全身到鞋；`精灵v1.4-24动作接触图.png` 与 `精灵v1.4-极限帧验收图.png` 为 v1.4 历史对照。

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

## 五、关键约定速记（期望这样做；偏离前先评估影响并跑对应门禁）

1. **唯一真相源是 `manifest.json`**：动作、帧数、尺寸、色数、节奏都从它生成，不要在 CSS / TS / 脚本里写第二份。
2. **每个动作一张专属 poster + 一张专属 loop**：禁止跨语义复用，禁止拖拽复用走路姿。
3. **每条 `.pet-loop[data-loop]` / `.pet-poster[data-frame]` 只能有一张 `background-image`**：CSS 多值是叠加不是 fallback，透明 WebP 会露出下层立绘成双影。
4. **整套替换**：换造型期望 51 张一起换（v1.4 为 49 张、v1.3.3 为 41 张，均属历史），只换静态海报会让同一 loop 里混进两套形象。
5. **拖拽是三段不是布尔**：按下 `lifted`（被提起来）→ 松手 `landing`（落地缓冲）→ `idle`；纯单击不播落地。
6. **云端 ImageGen / VideoGen 禁止进包**：无 alpha、带水印、跨图漂移；素材只走本地 ComfyUI。
7. **`--emit-only` 不是验收证据**：它跳过素材核对，只能用于生产中途生成常量。
8. **Git 操作前先报告并确认**：本项目约定，别顺手 commit。

---

## 六、放行清单（上线前逐条打勾）

- [ ] **【必处理 · CLEAN-001】历史误入库资产清理**：本轮按「本地素材 / 技能 / 开发向文档暂不入库」收口后，`git ls-files` 里仍有 333 份 `docs/孙测测/traces/` 留痕、测试报告/索引、约 15 份维护手册 `.md`、`AGENTS.md` 等开发向资产，以及 `.gitignore` 里整目录反忽略规则。期望在下次上线前用一次独立 `git rm --cached`（只删索引、不删本地文件）收口，并在干净 clone 上复跑 G-00~G-12 与 `npm run build`。**完整清单、保留项、反向遗留与验收方式见 `docs/孙测测/handoff/HANDOFF.md` §七**；本轮只登记、不处理。
- [ ] `petctl.py doctor` 全绿（**13 个**定版脚本副本 sha256 一致、master 与配置快照一致、**归一化基准与身高基线素材指纹一致**、**51 张 / `5898798 B` ≤8 MB**；v1.4 历史为 12 副本 / 49 张 / 5.670 MB，41 张属 v1.3.3 历史）
- [ ] `petctl.py frames` **v1.5 现行 154/154**（25 动作 1250 帧独立位图、四边留白 ≥6px、25 条 hitBox 按 `240×297` 与 alpha 并集一致、blink 与 sleep poster 字节不同）；v1.4 历史 148/148、20 动作时期 124/124
- [ ] `petctl.py metrics` **人物尺度量测通过**（跨动作身高极差 ≤8%、脚底中位同线、poster 与 loop 首帧同高、头身比与素材指纹对照；帧内波动等观察项照常留痕）
- [ ] `petctl.py check` 2/2（尺寸 / 帧数 / 时长 / alpha 与 manifest 一致；`pet_reencode_assets.py --check` 2/2）
- [ ] `petctl.py all`：gen 写模式 5/5、预览页重生成（内联 51 张、D05 同源）、frames 154/154、metrics 通过、`npm test` 全过 + `tsc --noEmit` 通过、`npm run build` 通过（dist **51 张**）、`git diff --check` 通过、**契约门禁 84/84**、新动作落包编码通过（C48 高度主导不变式为 v1.4 新增；`49 张 / 5536.62 KB / 148/148` 均属 v1.4 历史值，沿革 v1.3 76/76 → v1.3.2 78/78 → v1.3.3 83/83 → v1.4 84/84）
- [ ] 目视：`docs/孙测测/traces/assets/精灵v1.5-25动作接触图.png` 25 格人物同高、`精灵v1.4-极限帧验收图.png` 的 wake 双手完整、预览页 25 动作 + 拖拽三段（`精灵v1.4-24动作接触图.png` 24 格与 `v1.3.3最终素材接触图.png` 20 格仅作历史对照）
- [ ] 实窗口人工验收：按住被提起 / 松手落地 / 纯单击不落地 / 右键菜单 / 默认不穿透 / 跨屏 DPI / 动作切换不跳位 / **自主漂移像「被提起来飞过去」且落点自然落地**
- [ ] 许可口径：本地生成素材、中国大陆分发，MiniMax H3 Community License 地域限制已登记在 `ASSETS.md` 第六节
