# 小su 桌面精灵 · 进包素材留痕（v4 / v0.7 真实逐帧动画版）

生成日期：2026-11-08　|　编写：孙测测　|　生成方式：**全部本地 ComfyUI，零云端额度**

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

- **合计 11 张 / 404.23 KB**；`dist` 内 11 张同源，契约检查 D04 复核一致。
- 尺寸 240×360（@2x），CSS `background-size: contain`；5 个状态先按 alpha 包围盒对齐，切换不漂移。
- 每个 loop 由 RIFF 校验：`ANMF >= 2`、`VP8X` 的 `0x10`（alpha）与 `0x02`（animation）均置位；契约检查 D06 复核。
- 落包留痕：`docs/孙测测/traces/桌面精灵v0.7色键落包-*.md`；预览页生成留痕：`桌面精灵预览页生成-*.md`。

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
| 帧数 | 一期 37 帧 + H3 补间 | 5 状态 × 6 帧 = 30 帧 + 1 闭眼帧 | 6 帧闭环已覆盖呼吸/点头/轻摆，包体积可控（404 KB） |
| MiniMax-H3 | 补间产出进包素材 | **未用于进包素材**，保留给社区宣传短片 | 序列帧更可控；视频 alpha 跨平台解码风险高 |
| 动态载体 | 透明序列帧进包 | **带 alpha 的动画 WebP 进包**，CSS 只做交叉淡入与交互叠加 | 用户明确要求「不是图片叠加」；动画 WebP 由 ComfyUI 原生编码 |
| 闭眼 | Qwen-Edit 出 sleep 帧 | 程序化闭眼帧 + sleep loop 重编码 | Qwen-Edit 两轮闭眼指令均未生效 |
| 抠图 | rembg / 白底泛洪 | 洋红纯色底 + 色度空间软色键 | 本机缺 BiRefNet `Matting-HR.safetensors` 等权重；白底泛洪会留发丝缝隙白块、误伤白鞋白袜 |
| 互动 | 一期零互动 | 视线跟随、靠近光晕、随机眨眼、自主张望、轻碰回应、拖动倾斜 | v0.7 打磨项；写操作仍不进气泡，走确定性命令 + 后果预览 |

## 五、授权与合规

- 素材**全部由本地 ComfyUI 生成**，无云端图、无水印、无第三方素材混入，未消耗任何积分。
- 概念稿 `docs/design/assets/su-cel-*-concept.png` 为云端生成、带水印、无 alpha，**仅作画风参考，禁止入包**（本目录未引用）。
- **待核实**：Qwen-Image / Qwen-Image-Edit 权重及其 fp8 量化版、Lightning LoRA 各自的模型卡许可证是否允许随 Apache-2.0 仓库分发。宣传物料发布前需逐项确认；本目录资产不与代码许可证自动绑定。
