# 小su 桌面精灵 · 进包素材留痕

生成日期：2026-11-08　|　编写：孙测测　|　生成方式：**全部本地 ComfyUI，零云端额度**

## 一、产物清单

| 文件 | 状态 | 用途 | 体积 |
| --- | --- | --- | --- |
| `su-idle.webp` | idle / offline | 待机（offline 复用此帧 + 灰核心 + 降饱和） | 22.46 KB |
| `su-typing.webp` | typing | 真实请求在途时双手抬起敲键盘 | 25.38 KB |
| `su-done.webp` | done | 请求完成，抬手 | 21.93 KB |
| `su-error.webp` | error | 上游 4xx / 空响应 / 请求失败 | 20.10 KB |
| `su-sleep.webp` | sleep | 60s 无活动后闭眼休息 | 20.85 KB |

- 尺寸：360×560（@2x），CSS 显示为 180×280；RGBA 透明底，WebP q=82 / alpha_q=92。
- **合计 110.72 KB**。WebP 已是压缩格式，安装包 zip 二次压缩几乎不再缩小，故净增量 ≈ 110 KB。
- 落地留痕：`docs/孙测测/traces/桌面精灵素材落包-20261108-180909.md`。

## 二、生成链路（可复现）

服务：`http://192.168.31.84:8188/`（ComfyUI 0.36.0 / RTX 5080 Laptop 16GB / `--use-sage-attention`）

1. **唯一正面定稿**：`qwen_image_fp8_e4m3fn` + `Qwen-Image-Lightning-8steps-V1.0`，8 steps / cfg 1.0 / euler-simple，1024×1536，seed 取 `20261110`（三张候选中胸口六边形为纯空白，最适合运行时染色）。
2. **全部状态帧由图生图派生**：`qwen_image_edit_2509_fp8_e4m3fn` + `Qwen-Image-Edit-2509-Lightning-4steps-V1.0-bf16`，`TextEncodeQwenImageEdit` + `ModelSamplingAuraFlow(shift=3.0)` + `VAEEncode(定稿)`，4 steps / cfg 1.0。seed：`typing=20261110`、`done=20261111`、`error=20261112`、`sleep=20261113`、`idle=20261114`。
   - 不靠多次文生图碰一致性：一张定稿 + 一次编辑模型，五帧发型、服装、配色、构图完全一致。
3. **透明底**：本地脚本四边泛洪（中性浅色判据），不用颜色键，米白色外套不会被误抠；不依赖 Pillow / numpy。
4. **缩放编码**：ffmpeg Lanczos → 360×560 → cwebp。

复现命令（在仓库根目录）：

```bash
python3 docs/孙测测/scripts/comfyui_pet_assets.py master 20261110 20261111 20261112
python3 docs/孙测测/scripts/comfyui_pet_assets.py edit docs/design/assets/generated/pet-master-s20261110_00001_.png
python3 docs/孙测测/scripts/comfyui_pet_assets.py edit docs/design/assets/generated/pet-master-s20261110_00001_.png idle
python3 docs/孙测测/scripts/pet_frames_to_webp.py
```

提示词全文见 `docs/孙测测/scripts/comfyui_pet_assets.py` 的 `POSITIVE` / `NEGATIVE` / `EDIT_JOBS`。

## 三、与 v0.3 设计规格的偏差（如实记录）

| 项 | 规格 | 实际 | 原因 |
| --- | --- | --- | --- |
| 帧数 | 一期 37 帧 + H3 补间 | 5 帧 + CSS 动效 | 5 个状态各一帧已覆盖状态机；CSS 负责呼吸/浮动/敲键盘颤动/异常摇晃/完成弹跳，包体积从 2–3MB 降到 110KB |
| MiniMax-H3 | 补间产出进包素材 | **未用于进包素材**，保留给社区宣传短片 | 状态帧由图生图派生更可控、体积更小、无需处理视频 alpha 跨平台解码 |
| 全身 | 完整全身含脚 | 大半身（到大腿） | fp8 + 8steps 模型三轮稳定输出此构图；在 180×280 桌面尺寸下与全身无可见差别，全身反而更糊 |
| 光环 / 飘带 | S 形呆毛 + 数据流飘带 | **收编**：信号光环 + 翼状数据流飘带 | 三次重出 + 负向词均稳定出现，且视觉上更贴合「数字精灵」，故收编为正式设定 |
| 核心色 | 胸口六边形运行时染色 | 已实现：素材留白，CSS 六边形覆盖并按状态染色 | 换主题不用重出图 |
| 互动 | 一期零互动 | 一期 L1 只读：悬停看状态、点击回主窗口（需手动开启） | 写操作仍禁止进气泡，走既有确定性命令 + 后果预览 + `ModalPortal` |

## 四、已知瑕疵

- `su-sleep.webp` 的「闭眼」被图生图渲染成橙色实心圆，配合降饱和与慢呼吸后观感接近打盹，但并非标准闭眼线稿。
- `su-error.webp` 与 `su-idle.webp` 表情差异较小，异常态主要靠红色核心 + 摇晃动效 + error 气泡区分。
- 可在后续用 Qwen-Edit 单独重出这两帧替换，不影响代码。

## 五、授权与合规

- 素材**全部由本地 ComfyUI 生成**，无云端图、无水印、无第三方素材混入，未消耗任何积分。
- 概念稿 `docs/design/assets/su-cel-*-concept.png` 为云端生成、带水印、无 alpha，**仅作画风参考，禁止入包**（本目录未引用）。
- **待核实**：Qwen-Image / Qwen-Image-Edit 权重及其 fp8 量化版、Lightning LoRA 各自的模型卡许可证是否允许随 Apache-2.0 仓库分发。宣传物料发布前需逐项确认；本目录资产不与代码许可证自动绑定。
