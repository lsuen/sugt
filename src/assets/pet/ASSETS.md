# 小酥桌面精灵 · 进包素材留痕（v4 / v0.9 素材 + v1.0 互动层）

生成日期：2026-11-08（v0.8 / v0.9 / v1.0 增补 2026-11-09）　|　编写：孙测测　|　生成方式：**全部本地 ComfyUI，零云端额度**

> v4 修订（对应「头发没抠干净 / 在想时奇怪摇晃 / 别只是图片叠加」三轮反馈）：
> - **放弃白底泛洪**：改 Qwen-Edit 均匀洋红纯色底 + 色度空间软色键，发丝缝隙是纯背景色，必然 `alpha=0`；深/浅桌面下无白块、无粉边，白鞋白袜无破洞。因色键已能干净处理发丝，**未改白头发**，保持雾蓝短发设定。
> - **真实逐帧动画**：5 个状态各 6 帧 / 8fps、带 alpha 的动画 WebP，由 ComfyUI `SaveAnimatedWEBP` 编码，替代 v0.6 的静态图 + CSS 无限旋转。
> - **闭眼帧程序化生成**：Qwen-Edit 闭眼指令不稳定，改为按实测眼位局部覆盖 + 睫毛弧线，sleep 循环也由闭眼帧重编码，不再有「睁眼睡觉」瑕疵。
> - 前置能力核实见 `docs/孙测测/traces/桌面精灵v0.7能力核实与方案-20261108.md`。

## 一、产物清单

| 文件 | 类型 | 用途 | 体积 |
| --- | --- | --- | --- |
| `su-idle.webp` | 静态海报 | reduced-motion / 窗口隐藏时降级显示 | 10.52 KB |
| `su-typing.webp` | 静态海报 | 同上（处理中） | 10.13 KB |
| `su-done.webp` | 静态海报 | 同上（已完成） | 9.61 KB |
| `su-error.webp` | 静态海报 | 同上（出错了） | 10.08 KB |
| `su-sleep.webp` | 静态海报 | 同上（休息中，闭眼） | 8.69 KB |
| `su-blink.webp` | 静态海报 | 运行时随机眨眼叠加层（程序化闭眼帧） | 10.37 KB |
| `su-idle-loop.webp` | 动画 WebP | idle 循环，6 帧 / 8fps / alpha | 72.29 KB |
| `su-typing-loop.webp` | 动画 WebP | typing 循环，6 帧 / 8fps / alpha | 69.40 KB |
| `su-done-loop.webp` | 动画 WebP | done 循环，6 帧 / 8fps / alpha | 66.18 KB |
| `su-error-loop.webp` | 动画 WebP | error 循环，6 帧 / 8fps / alpha | 69.21 KB |
| `su-sleep-loop.webp` | 动画 WebP | sleep 循环，6 帧 / 8fps / alpha（闭眼） | 67.74 KB |
| `su-wave.webp` | 静态海报 | 轻碰招手姿（v0.8） | ~9–11 KB |
| `su-peek.webp` | 静态海报 | 靠近倾听姿（v0.8） | ~9–11 KB |
| `su-shy.webp` | 静态海报 | 害羞姿（v0.9） | ~10 KB |
| `su-celebrate.webp` | 静态海报 | 庆祝姿（v0.9） | ~9 KB |
| `su-walk.webp` | 静态海报 | 轻步靠近姿（v0.9） | ~9 KB |
| `su-wave-loop.webp` | 动画 WebP | wave 循环 | ~73 KB |
| `su-peek-loop.webp` | 动画 WebP | peek 循环 | ~72 KB |
| `su-shy-loop.webp` | 动画 WebP | shy 循环 | ~74 KB |
| `su-celebrate-loop.webp` | 动画 WebP | celebrate 循环 | ~68 KB |
| `su-walk-loop.webp` | 动画 WebP | walk 循环 | ~69 KB |

- **v0.7 基线 + v0.8 wave/peek + v0.9 shy/celebrate/walk；v1.0 未新增任何素材**，21 张清单保持不变。尺寸 240×360（@2x），切换前按 alpha 包围盒对齐。
- **实测口径（2026-11-09，P0 历史值）**：10 个动画 WebP 各 **6 帧 × 125ms = 750ms / 8fps**，合计 60 帧；21 张素材合计 **807.6 KB**。上表体积为 v0.7 首版留痕值，静态海报后续重编码过。

- **v1.1 修订（2026-11-09 晚，现行口径以 §八 为准）**：以上与 §四、§七 的 `240×360` / 6 帧 / 807.6 KB 均为 P0 历史口径，仅作留痕保留。当前落包为 `105×297 @2x`、每姿势 10 帧往返、非匀速逐帧时长、96 色 palette-lossless，21 张合计 **677.3 KB / 100 帧**。
- 每个 loop 由 RIFF 校验：`ANMF >= 2`、`VP8X` alpha + animation 置位；逐帧时长与帧数由 `manifest.json` + `gen_pet_from_manifest.py` 机械校验。
- **唯一真相源**：`manifest.json`（姿势清单、素材文件名、帧数、节拍、播放模式、逐帧 alpha 包围盒）。CSS 生成段、`src/pet/petManifest.generated.ts`、预览脚本与契约脚本都从它派生，禁止再手工维护第二份姿势清单。
- 落包留痕：`docs/孙测测/traces/桌面精灵v0.7色键落包-*.md`（含 20261109 表情库）。
- **交互**：穿透模式下靠 `pet_global_cursor` 轮询全局光标驱动视线/靠近；勿再假设 document `pointermove` 在 ignore_cursor_events 下可用。
- **元素级穿透**：只有光标落在立绘或右键菜单命中框内（或正在拖动）才 `pet_set_clickable(true)`；空白侧栏与透明边距永远穿透到桌面。禁止「菜单一开就整窗解穿透」——那会把透明窗矩形变成挡点击的透明墙。
- **零底色铁律**：产品窗禁止任何舞台底板（无 `.pet-halo` / `.pet-ground`、无 `.pet-stage` 纯色/径向铺底）。`overlay.html` 的 `html,body,#root` 与 `.pet-stage` 必须 `background: transparent !important`。演示页可用棋盘格模拟壁纸，但虚线框内仍须透明。
- **v1.0 互动层映射**：开场招呼 / 轻碰 → `wave`，靠近倾听 → `peek`，摸头 → `shy`，连点三下 / 完成态 → `celebrate`，自主走动 → `walk`，闲置差分从上述五种随机取一。新增的心形、思考云、落地回弹、步态起伏全部由 CSS/JS 生成，**不新增素材、不新增动画层**。

## 二、生成链路（可复现）

服务：`http://192.168.31.84:8188/`（ComfyUI 0.36.0 / RTX 5080 Laptop 16GB）

1. **洋红底基准图**：Qwen-Edit 把 v2 白底全身定稿的背景改成均匀洋红纯色，得 `v4/pet-chroma-base.png`；实测背景中位色 `(255, 62, 166)`，边界样本均匀。
2. **状态差分帧**（`comfyui_pet_v07.py states`）：`qwen_image_edit_2509_fp8_e4m3fn` + Lightning 4steps LoRA，4 steps / cfg 1.0；从洋红底重出 typing / done / error / sleep。`EDIT_BASE` 强制「背景保持同一块纯净洋红纯色」「只有两条手臂」「正面全身站姿」。
3. **色键落包**（`pet_v07_frames.py`）：
   - 色度空间软色键：以 `(R-G, B-G)` 到背景色的距离定 alpha，`CHROMA_FLOOR=14` / `CHROMA_FULL=180`；再做**反预乘去溢色**，消除洋红描边。
   - `clean_specks()` 连通域清噪，`SPECK_MIN_AREA=48`，清掉 error 帧的游离呆毛碎块。
   - `alpha_bbox` / `median_box` / `warp_to_box` 对齐，消除 Qwen-Edit 构图漂移。
   - `make_closed_eyes()` 按实测眼位 `EYE_BOXES = ((102,83,117,97),(124,83,139,97))` 局部羽化覆盖为肤色并画向下弯弧睫毛线，产 `su-blink.webp`；**不用 CSS 贴片眼睑**（会盖住刘海，像蒙眼布）。
   - `motion_frames()` 生成 6 帧真实差异帧、首尾闭合，供动画编码。
4. **ComfyUI 动画编码**（`comfyui_pet_v07.py anim`）：上传 RGBA 序列帧 → `LoadImage` → **`JoinImageWithAlpha` 还原 alpha** → `ImageBatch` → `SaveAnimatedWEBP`（`fps=8` / `quality=88` / `lossless=False`，WebP 的 `ALPH` 块独立压缩，仍保留透明通道）→ 剥离 EXIF → RIFF 结构校验。

复现命令（仓库根目录）：

```bash
# 状态差分帧（输出默认落到 docs/design/assets/generated/v4，可用 SUGT_PET_OUT 覆盖）
python3 docs/孙测测/scripts/comfyui_pet_v07.py states \
  docs/design/assets/generated/v4/pet-chroma-base.png typing done error sleep
# 色键 → RGBA → 静态海报 + 序列帧（默认写到 src/assets/pet 与 v4/frames）
python3 docs/孙测测/scripts/pet_v07_frames.py
# 序列帧 → 带 alpha 动画 WebP
python3 docs/孙测测/scripts/comfyui_pet_v07.py anim \
  docs/design/assets/generated/v4/frames
# 同源预览页与契约复核
python3 docs/孙测测/scripts/build_pet_preview.py
python3 docs/孙测测/scripts/check_pet_overlay.py
```

## 三、两个必须避开的坑（能力核实已确认）

- **`SaveAnimatedWEBP` 会对相同帧去重**：用 `RepeatImageBatch` 重复 4 张同图，产出的是静态单帧（`VP8X(0x18)`、无 `ANIM`、`ffprobe nb_read_frames=1`）。因此动画必须由**真实差异帧**组成，不能靠 CSS 叠图或复制帧。
- **`LoadImage` 会把 RGBA 拆成 `IMAGE(RGB) + MASK(alpha)`**：编码前不 `JoinImageWithAlpha` 还原，动画 WebP 会变成洋红 / 黑底。已用独立 RGBA 探针验证 `SaveImage` 输出 `pix_fmt=rgba`。

## 四、与 v0.3 设计规格的偏差（如实记录）

| 项 | 规格 | v0.7 实际 | 原因 |
| --- | --- | --- | --- |
| 帧数 | 一期 37 帧 + H3 补间 | **10 姿势 × 6 帧 = 60 帧** + 11 张静态海报（含 1 张程序化闭眼帧） | v0.7 首版为 5 状态 × 6 帧 = 30 帧 / 404 KB；v0.8/v0.9 补齐 5 个差分姿势后为 60 帧 / 807.6 KB。6 帧闭环只有呼吸/点头/轻摆，0.75s 循环导致体感抖动，扩容评估见 `docs/design/桌宠素材扩容评估.md` |
| MiniMax-H3 | 补间产出进包素材 | **未用于进包素材**，保留给社区宣传短片 | 序列帧更可控；视频 alpha 跨平台解码风险高 |
| 动态载体 | 透明序列帧进包 | **带 alpha 的动画 WebP 进包**，CSS 只做交叉淡入与交互叠加 | 用户明确要求「不是图片叠加」；动画 WebP 由 ComfyUI 原生编码 |
| 闭眼 | Qwen-Edit 出 sleep 帧 | 程序化闭眼帧 + sleep loop 重编码 | Qwen-Edit 两轮闭眼指令均未生效 |
| 抠图 | rembg / 白底泛洪 | 洋红纯色底 + 色度空间软色键 | 本机缺 BiRefNet `Matting-HR.safetensors` 等权重；白底泛洪会留发丝缝隙白块、误伤白鞋白袜 |
| 互动 | 一期零互动 | v1.0：头转向鼠标（rAF 插值）、开场招呼、闲置差分、唤醒、摸头、连点庆祝、屏幕内自主走动、落地回弹、脚底状态胶囊 | 复用 v0.9 十姿势素材，纯前端状态机 + 后端夹边；写操作仍不进气泡，走确定性命令 + 后果预览 |

## 五、v1.0 互动层验收红线（2026-11-09）

| 红线 | 落地位置 | 验收方式 |
| --- | --- | --- |
| 素材仍为 21 张，零新增、零云端 | 本目录 + `ASSETS.md` 清单 | `check_pet_overlay.py` C17 / D04 / D06 |
| 每个姿势只挂一张背景（透明区不叠第二个人影） | `src/pet/pet.css` 单层规则 | C17 逐条断言 urls 恰为单元素 |
| 请求级只播角色动作，不出泡；失败必切 error | `src/pet/PetStage.tsx` 状态跃迁 | C05 / C19 + 契约留痕 |
| 走动只在 idle 且久无操作时，在当前显示器可用区内随机落点缓动；左右朝向 CSS 镜像，不新增四向素材 | `src/pet/hooks/usePetBehaviors.ts` + `src-tauri/src/overlay.rs::roam_to` | C20 / C20b / C20c + `cargo test --lib` 夹边单测 |
| 免打扰只静音气泡，动作与状态仍可切换 | `pushBubble` 统一收口 | 设置页 `set.petQuiet` + 预览页气泡开关 |
| 预览页与源 CSS 同源、含 v1.0 互动演示 | `docs/design/pet-v07-preview.html` | D05 / D07 / D08 + 浏览器冒烟 12 项 |

> 2026-11-09 追加：演示页去掉 srcdoc iframe，精灵 DOM 与按钮在同一文档，避免预览面板给 iframe 分配 opaque origin 时 `contentDocument` 不可用导致「按钮只亮一下、角色没反应」。互动按钮改为触发式动作（1.5~2.6s：换姿势 + 气泡 + 脚底状态文字 + 位移），播完自动复位、连点可重播，另提供自动演示与点角色本体；旧版 0.3~0.7s 一次性动画播完即静止是这次缺陷的根因。为同文档渲染，`.pet-mote` / `.pet-spark` 从 `vw` 改为固定 `2px` / `1.6px`（与真实 148~220px 悬浮窗等价），避免粒子被整页视口放大近 8 倍。

## 六、授权与合规

- 素材**全部由本地 ComfyUI 生成**，无云端图、无水印、无第三方素材混入，未消耗任何积分。
- 概念稿 `docs/design/assets/su-cel-*-concept.png` 为云端生成、带水印、无 alpha，**仅作画风参考，禁止入包**（本目录未引用）。
- **待核实**：Qwen-Image / Qwen-Image-Edit 权重及其 fp8 量化版、Lightning LoRA 各自的模型卡许可证是否允许随 Apache-2.0 仓库分发。宣传物料发布前需逐项确认；本目录资产不与代码许可证自动绑定。
- **扩容硬前置**：上条未闭环前不得生成/替换进包素材；若扩容时引入 H3 / RIFE 等补间模型，同样逐项核实许可证。`docs/design/桌宠素材扩容评估.md` §5 列为阻断项。

## 七、契约化与扩容（2026-11-09 P0 落地）

| 改动 | 位置 | 作用 |
| --- | --- | --- |
| 素材契约单一真相源 | `manifest.json` | 10 姿势的 poster/loop 文件名、帧数、逐帧时长、fps、playback、`hitBox` |
| 生成器 + 门禁 | `docs/孙测测/scripts/gen_pet_from_manifest.py` | 校验 RIFF 帧数/时长/alpha，离线算逐帧 alpha bbox 并回填，生成 CSS 段与前端常量；`--check` 生成物过期即失败 |
| 前端常量 | `src/pet/petManifest.generated.ts` | `PET_POSE_NAMES` / `PET_HITBOXES` / `PET_ASSET_FILES` / `PET_FRAME_MS` / `PET_IMAGE_SIZE` |
| 状态机 | `src/pet/petState.ts` | 声明式 `PET_STATE_TABLE` / `PET_POSE_TABLE`（行序即优先级）+ 纯 `reducePetState` |
| 穿透命中区 | `src/pet/config.ts::figureHitRect`、`hooks/usePetGaze.ts` | 按当前姿势的 alpha 包围盒收紧，透明边距不再变成点击墙 |

验收命令：

```bash
npm test                                        # 含 test:pet-state / test:pet-manifest，跨平台
npm run build
python3 docs/孙测测/scripts/gen_pet_from_manifest.py --check   # 需 ffmpeg
python3 docs/孙测测/scripts/build_pet_preview.py
python3 docs/孙测测/scripts/check_pet_overlay.py
```

> 本轮**未新增、未替换任何素材**，21 张 / 60 动画帧保持不变；扩容规模与连贯播放下限见 `docs/design/桌宠素材扩容评估.md`（推荐 B+ 档 252 帧 / 12fps / 2–3s / 约 3.1MB，先扩 idle/typing/done 三姿势验证体感）。该 B+ 建议已被 §八 的 v1.1 实测结论取代。

---

## 八、v1.1 裁剪重编码与自动交互（2026-11-09，孙测测）

### 8.1 当前落包口径（本节为唯一现行值）

| 项 | v1.0 / P0 | **v1.1 现行** |
| --- | --- | --- |
| 画布 | `240×360 @2x` | **`105×297 @2x`**（`cropBox = [68,34,173,331]`） |
| 每姿势帧数 | 6 帧 / 125ms / 8fps / 0.75s | **10 帧往返**（`frameOrder = [0,1,2,3,4,5,4,3,2,1]`）/ 非匀速 `frameMs[]` / 约 1.18s |
| 编码 | ComfyUI `SaveAnimatedWEBP` q88 有损 | **96 色量化 + lossless WebP**（`method=4`、`dither=none`），仍是 `VP8X(0x12)+ANIM+ANMF` 带 alpha |
| 人物占比 | alpha 包围盒约占画布宽 **38%** | 约占 **87%**，透明点击墙消除 |
| 素材数 / 体积 | 21 张 / 60 动画帧 / 807.6 KB | **21 张 / 100 动画帧 / 693508 B（677.3 KB）** |
| 尺寸档 | 148×168 / 180×204 / 220×250 | **120×136 / 148×168 / 180×204**（旧大档废弃，新增更小档，`PET_SIZE_STORAGE_KEY` 不变） |
| 穿透 | 默认整窗穿透 + 元素级解穿透 | **`pet_click_through` 默认 `false`**，整窗可交互；仅开关开启且 `interactive && !edit` 才走元素级穿透 |

复现命令（仓库根目录）：

```bash
/usr/local/bin/python3.13 docs/孙测测/scripts/pet_reencode_assets.py --source /tmp/sugt-pet-original
python3 docs/孙测测/scripts/pet_reencode_assets.py --check       # 纯标准库 RIFF，可进 CI
python3 docs/孙测测/scripts/gen_pet_from_manifest.py             # 重生成 CSS 段与 TS 常量
python3 docs/孙测测/scripts/build_pet_preview.py
npm test && npm run build
python3 docs/孙测测/scripts/check_pet_overlay.py                 # 67/67
```

> 写模式依赖 Pillow（本机 `/usr/local/bin/python3.13` 为 11.2.1 且 `webp_anim=True`）；系统默认 `python3` 无 PIL。`--source` 必须指向**未经裁剪的原始 240×360 帧**，重复对已裁剪素材执行会二次裁切。留痕：`traces/精灵素材重编码-20261109-231406.md`、`traces/精灵产物校验-20261109-234026.md`。

### 8.2 裁剪收益实测（修正「裁透明边能大幅省体积」的假设）

| 口径 | 体积 | 结论 |
| --- | --- | --- |
| 原 240×360 / q82 有损 | 827004 B | 基线 |
| 裁剪后 105×297 / q82 有损 | 748244 B | **仅省 9.5%**——全透明像素在 WebP 里几乎不占字节 |
| 裁剪 + 96 色 palette-lossless | **693508 B** | 体积收益主要来自量化 + lossless，不是裁剪 |

裁剪的真实价值是**命中区与窗口尺寸**：`hitBox` 从 `[0.3125,0.1111,0.6917,0.8944]` 变为约 `[0.0667,0.0202,0.9333,0.9697]`，透明边距不再吞掉拖拽/右键，窗口得以整体下移一档。

### 8.3 1000 帧体积预算（实测外推，非估算）

以现行 100 帧 / 693508 B 的**单帧均价 6935 B** 外推，p96-lossless 1000 帧约 **6.94 MB**（已确认，来自 `pet_reencode_assets.py --check` 实测口径）。p64-lossless 约 4.6 MB、p128-lossless 约 7.1 MB、q82 有损约 10.7 MB 为同批素材的对照试跑值（见 `docs/design/桌宠素材扩容评估.md` §3，属**外推推断**，未做完整 1000 帧落包）。**结论：1000 帧在 8 MB 内可行，5 MB 目标需降到 64 色**；默认保持 96 色以护住赛璐璐平涂色块。

### 8.4 自动交互动作（借 whale-girl 状态机，不搬素材）

`manifest.autoActions` 为节奏唯一真相源，`src/pet/petState.ts` 提供纯节奏器（不读 `Date.now()`、不设定时器）：眨眼 3–9s、随机转身 10–25s、闲置差分 12–30s（hold 2.2s，从 `peek/wave/shy/walk` 取）、游走 18–40s、开场招呼 6s、完成庆祝 3.5s、失败失落 3s、睡醒过渡 1.6s、拖拽态与等待槽位。优先级 `drag > wake > welcome > celebrate > disappointed > ... > idle`；**零负反馈**，失败只是一次短情绪，不惩罚、不落盘。`pet.css` 的 `data-motion` keyframes 只做 CSS 位移/倾斜，C17 仍逐条断言每个姿势单层 `background-image`。

### 8.5 尚未落地与阻断项（如实记录）

- **本轮 100 帧仍是「6 帧差异帧 + 往返帧序」的工程改造**，不是新动作素材。C 档 360 帧 / 1000 帧真动作（转头、伸懒腰、挥手）未生产：仿射补间无法凭空造出动作语义，必须由唯一正面定稿经 Qwen-Image-Edit 出关键姿态 → 本地 ComfyUI 补间 → 色键对齐 → 本脚本编码 → 全门禁。
- **合规硬前置不变**：§六 的 Qwen-Image / Qwen-Image-Edit / Lightning LoRA 许可证，以及新增的 RIFE / FILM / H3 补间模型许可证，未逐项闭环前**不得生成或替换进包素材**；双平台 lossless 动画 WebP 播放亦需冒烟。
- 实窗口人工验收（默认不穿透下的拖拽 / 右键菜单 / 跨屏 DPI / 10 帧连贯感）**未执行**，当前只有契约、单测与浏览器级证据。
