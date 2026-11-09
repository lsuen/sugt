# MAP — 脚本 / 检查项 / 留痕 / 报告映射表

> **维护人**：孙测测　|　**适用项目**：SUGT　|　**建立日期**：2026-11-06
> **用途**：一张表查清「哪个脚本能复用、它覆盖什么、跑完的证据在哪、对应哪份版本报告」，避免重复造脚本、重复读源码。

---

## 一、脚本能力索引

| 脚本编号 | 脚本路径 | 能力分类 | 覆盖检查项 | 主要输入 | 产出留痕 | 关联版本报告 | 状态 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| SCR-001 | `scripts/check_version_consistency.py` | 配置管理 / 白盒 | `package.json`、`Cargo.toml`、`Cargo.lock`、`tauri.conf.json` 四处版本号一致性（7 键） | `--root` `--expect` `--out` | `traces/版本一致性检查-<时间戳>.md` | 所有 `sugt*测试报告NN.md` 的报告头「版本/构建号」 | 已落地、已自测 |
| SCR-002 | `scripts/comfyui_qwen_smoke.py` | 素材链路 / 环境 | 本地 ComfyUI 连通性、模型与 LoRA 可用性冒烟 | `--host` `--out` | `traces/ComfyUI-Qwen定稿冒烟-<时间戳>.md` | 精灵素材落包留痕 | 已落地 |
| SCR-003 | `scripts/comfyui_pet_assets.py` | 素材链路 / 生成 | Qwen 定稿 → Qwen-Edit 状态帧 → H3 短片的关键帧生产 | `--host` `--out` | `traces/桌面精灵素材落包-<时间戳>.md` | 精灵素材落包留痕 | 已落地 |
| SCR-004 | `scripts/pet_frames_to_webp.py` | 素材链路 / 后处理 | 抽帧、纯白严格抠图、alpha bbox 几何对齐、@2x 透明 WebP 落包 | `--root` `--out` | `traces/桌面精灵素材落包-<时间戳>.md` | 精灵素材落包留痕、`src/assets/pet/ASSETS.md` | 已落地 |
| SCR-005 | `scripts/build_pet_preview.py` | 前端 UI / 可视化 | 从 `src/overlay.css` 抽同源样式 + 内联 11 张素材（含 5 个动画 WebP）生成离线预览页，校验 13 个关键选择器与 iframe 脚本闭合 | `--root` `--out` `--trace-dir` | `traces/桌面精灵预览页生成-<时间戳>.md`、`docs/design/pet-v07-preview.html` | v0.7 视觉验收 | 已落地 |
| SCR-006 | `scripts/check_pet_overlay.py` | 白盒架构 / 契约 | 精灵源码修复点、ACL、i18n 双语、dist 产物、预览同源、动画 WebP RIFF 结构、iframe 脚本闭合共 28 项可判定检查 | `--root` `--out` | `traces/桌面精灵链路契约检查-<时间戳>.md` | v0.7 复验报告回归结论 | 已落地 |
| SCR-007 | `scripts/comfyui_pet_v07.py` | 素材链路 / 生成 | 本地 ComfyUI 洋红底状态差分帧 + `SaveAnimatedWEBP` 带 alpha 动画编码 + EXIF 剥离与 RIFF 校验 | `states\|anim <源>`、`SUGT_PET_OUT` / `SUGT_COMFYUI_BASE` | v4 状态帧 / 序列帧 / loop WebP、`traces/桌面精灵v0.7色键落包-*.md` | v0.7 素材链路 | 已落地 |
| SCR-008 | `scripts/pet_v07_frames.py` | 素材链路 / 后处理 | 洋红底色度空间软色键、反预乘去溢色、连通域清噪、alpha bbox 对齐、程序化闭眼帧、6 帧首尾闭合差异序列 | `--src` `--out` `--frames` `--trace` `--size` `--count` | `traces/桌面精灵v0.7色键落包-<时间戳>.md` | `src/assets/pet/ASSETS.md` | 已落地 |
| SCR-009 | `scripts/pet_v07_browser_smoke.py` | 前端 UI / 浏览器 | Playwright 真实时间采样：动画 WebP 逐帧播放、视线跟随与靠近光晕、眨眼帧、静态降级、状态切换、轻碰反馈、验收截图 | `--root` `--out`、`SUGT_PLAYWRIGHT_MODULE` / `SUGT_CHROME_PATH` | `traces/桌面精灵v0.7浏览器冒烟-<时间戳>.md` + 同名 PNG | v0.7 复验报告 UI 结论 | 已落地 |
| SCR-010 | `scripts/gen_pet_from_manifest.py` | 白盒架构 / 素材契约 | manifest v2 ↔ 素材（ANMF 帧数 / `frameMs[]` 逐帧时长 / alpha 标志 / 逐帧 alpha bbox）↔ CSS 生成段 ↔ 前端常量；`--check` 生成物过期即失败 | `--root` `--check` `--out`，需 ffmpeg | `traces/精灵Manifest生成物校验-<时间戳>.md` | 桌宠技术栈对比方案 P0-1 / P0-3；扩容评估 §4 | 已落地、已自测 |
| SCR-011 | `scripts/pet_reencode_assets.py` | 素材链路 / 后处理 | 按 `cropBox` 裁透明边、按 `frameOrder` 物化往返帧、量化到 `encoding.colors` 色后 lossless 动画 WebP 重编码；`--check` 纯标准库解析 RIFF 校验尺寸 / 帧数 / 逐帧时长 / alpha 并估算 1000 帧体积 | `--root` `--source` `--check` `--out`，写模式需 Pillow | `traces/精灵素材重编码-<时间戳>.md`、`traces/精灵产物校验-<时间戳>.md` | v1.1 裁剪与 1000 帧预算实测；`ASSETS.md` §八 | 已落地、已自测 |

| SCR-012 | `scripts/comfyui_curl_http.py` | 素材链路 / 传输层 | 本机 Python socket 直连内网 ComfyUI 报 `Errno 65 No route to host` 而 curl 可达；monkey-patch `urllib.request.urlopen` 为 curl 通道，只换传输层不改 workflow | 模块内 `install()`，幂等 | 无独立留痕（由调用脚本留痕） | v1.2 ComfyUI 能力核实前置 | 已落地、已自测 |
| SCR-013 | `scripts/comfyui_pet_keyposes_trial.py` | 素材链路 / 能力核实 | 复用 v0.7 Qwen-Image-Edit 链路，对转头 / 歪头 / 伸懒腰等新动作各出一张关键姿态，产物只落临时目录 | `--ref` `--actions` `--out`，禁止写入 `src/assets/pet` | `traces/精灵关键姿态试片-<时间戳>.md` | 1000 帧真动作素材硬前置 | 已落地；首轮能力核实**不通过**（见留痕） |
| SCR-014 | `scripts/comfyui_pet_palette_trial.py` | 素材链路 / 形象决策 | 图生图派生银灰发 / 街舞工装裤 / 暖色对冲三套形象候选，产物只落临时目录 | `--ref` `--schemes` `--out`，禁止写入 `src/assets/pet` | `traces/精灵形象候选试片-<时间戳>.md` | 撞色核实与形象定稿窗口 | 已落地；首轮**不通过**（Edit 链路不执行外观级改动） |

> 后续新增脚本按 `SCR-015` 起递增登记，**先登记再写脚本**，保证索引不落后于实现。
> 跨平台分工：SCR-010 需 ffmpeg、SCR-011 写模式需 Pillow，两者只在本地素材链路跑；SCR-011 的 `--check` 为纯标准库可进 CI。Windows CI 的等价守卫是 `npm run test:pet-manifest`（纯 Node，校验 manifest v2 ↔ CSS ↔ 生成 TS 语义一致）。

## 二、检查项 ↔ 能力矩阵

| 检查项 | 功能 | 业务 | 前端 UI | 白盒架构 | 现有自动化 | 脚本覆盖 |
| --- | --- | --- | --- | --- | --- | --- |
| 版本号一致性与可追溯 | — | ✓ | — | ✓ | — | SCR-001 |
| 主题契约（token / 选择器 / 注册表） | — | — | ✓ | ✓ | `npm test`（主题校验） | 待补 SCR-002 |
| Rust 全量套件汇总 | — | — | — | ✓ | `cargo test` | 待补 SCR-002 |
| Responses 网关工具链路 | ✓ | ✓ | — | ✓ | `codex_gateway_e2e.rs` / `codex_edge_cases.rs` | 复用现有，不重写 |
| 真机日志异常扫描 | ✓ | — | — | ✓ | — | 待补 SCR-003 |

## 三、留痕索引规则

- 留痕由脚本**自动**写入 `traces/`，命名 `<检查项>-<YYYYMMDD-HHMMSS>.md`，只增不改。
- 每份留痕必须包含：执行时间、输入参数、逐项结果、**结论**、复核命令 —— 任何人可独立复跑验证。
- 版本报告中引用证据时，写「留痕文件名 + 结论」，不把大段日志贴进报告。
- 历史留痕清单：见下方「四、执行记录」追加区，按时间倒序追加，不覆盖。

## 四、执行记录（追加区）

| 日期时间 | 脚本 | 结论 | 留痕文件 | 备注 |
| --- | --- | --- | --- | --- |
| 2026-11-06 20:01:37 | SCR-001 | 通过（四处均 1.1.7，退出码 0） | `traces/版本一致性检查-20261106-200137.md` | 首轮自测，默认参数 |
| 2026-11-06 20:01:49 | SCR-001 | 不通过（期望 1.1.8 ≠ 实际 1.1.7，退出码 1） | `/tmp/sugt-selftest-expect/` | 负向用例自测，留痕不入库 |
| 2026-11-06 20:01:49 | SCR-001 | 不通过（文件缺失，退出码 2） | `/tmp/sugt-selftest-missing/` | 异常路径自测，留痕不入库 |
| 2026-11-06 23:30:21 | SCR-001 | 通过（四处均 1.1.7，退出码 0） | `traces/版本一致性检查-20261106-233021.md` | 1.1.8 打版前基线（脚本当时仍只查四处） |
| 2026-11-06 23:38:57 | SCR-001 | 通过（7 键均 1.1.8，退出码 0） | `traces/版本一致性检查-20261106-233857.md` | 脚本扩展到 7 键后打版终检；证据见 `docs/TEST_REPORT.md` 第 18 节 |
| 2026-11-07 00:28:13 | SCR-001 | 通过（7 键均 1.1.8，退出码 0） | `traces/版本一致性检查-20261107-002813.md` | 封版复验；release run 42250535758 全绿后 |
| 2026-11-08 20:23:33 | SCR-005 | 通过（5 张素材内联、7 个选择器齐全，退出码 0） | `traces/桌面精灵预览页生成-20261108-202333.md` | v0.6 首版预览页 |
| 2026-11-08 20:24:55 | SCR-005 | 通过（新增 URL hash 组合验收，退出码 0） | `traces/桌面精灵预览页生成-20261108-202455.md` | 大档/出错态/编辑态/浅色桌面批量截图 |
| 2026-11-08 20:26:53 | SCR-006 | 通过（19/19，退出码 0） | `traces/桌面精灵链路契约检查-20261108-202653.md` | v0.6 修复后回归 |
| 2026-11-08 20:27:02 | SCR-001 | 通过（7 键均 1.1.9，退出码 0） | `traces/版本一致性检查-20261108-202702.md` | 本轮未改版本，确认无漂移 |
| 2026-11-08 21:52:02 | SCR-005 | 通过（11 张素材内联、13 个选择器齐全，退出码 0） | `traces/桌面精灵预览页生成-20261108-215202.md` | 修复 srcdoc 结束标签双重转义后重新生成，iframe 视线脚本恢复执行 |
| 2026-11-08 21:52:17 | SCR-009 | 通过（7/7，退出码 0） | `traces/桌面精灵v0.7浏览器冒烟-20261108-215217.md` | 动画 WebP 7 次采样 6 个不同帧；视线 gaze-x -0.829→0.237、贴头 near 0.999 / nearby=true |
| 2026-11-08 21:52:36 | SCR-001 | 通过（7 键均 1.1.9，退出码 0） | `traces/版本一致性检查-20261108-215236.md` | v0.7 未改版本，确认无漂移 |
| 2026-11-08 21:53:55 | SCR-006 | 通过（28/28，退出码 0） | `traces/桌面精灵链路契约检查-20261108-215355.md` | 新增 D07 预览页 iframe 脚本闭合检查；D06 五个 loop 均 6 帧 / alpha |
| 2026-11-09 22:41:00 | SCR-010 | 通过（5/5，退出码 0） | `traces/精灵Manifest生成物校验-20261109-224100.md` | 写模式：回填 10 个姿势实测 alpha hitBox，生成 `petManifest.generated.ts` |
| 2026-11-09 22:41:46 | SCR-010 | 通过（5/5，退出码 0） | `traces/精灵Manifest生成物校验-20261109-224146.md` | `--check` 负向复验：素材契约与生成物均无漂移 |
| 2026-11-09 22:41:46 | SCR-005 | 通过（21 张 / 807.6 KB，退出码 0） | `traces/桌面精灵预览页生成-20261109-224146.md` | CSS 生成段改动后重生成，D05 同源 hash `887c82224c7f` |
| 2026-11-09 22:42:36 | SCR-006 | 通过（56/56，退出码 0） | `traces/桌面精灵链路契约检查-20261109-224236.md` | 新增 C25/C33/C34/C35；`npm test` / `npm run build` / `cargo test --lib`（271 通过）同期全绿 |
| 2026-11-09 23:14:06 | SCR-011 | 通过（827004 B → 693508 B / 100 帧，退出码 0） | `traces/精灵素材重编码-20261109-231406.md` | v1.1 裁透明边 + 10 帧往返 + 96 色 palette-lossless 落包，实测 1000 帧约 6.94 MB |
| 2026-11-09 23:30:05 | SCR-005 | 通过（21 张 / 677.3 KB，退出码 0） | `traces/桌面精灵预览页生成-20261109-233005.md` | v1.1 三档尺寸 + 10 段自动交互演示，D05 同源 hash `94861738bf27` |
| 2026-11-09 23:39:36 | SCR-006 | 通过（67/67，退出码 0） | `traces/桌面精灵链路契约检查-20261109-233936.md` | C37 断言 needle 由 `height: 204,` 修正为 `height: 204` 后复验；本轮新增 C36~C40 / D08 共 11 项 |
| 2026-11-09 23:40:25 | SCR-010 | 通过（5/5，退出码 0） | `traces/精灵Manifest生成物校验-20261109-234025.md` | manifest v2 生成物最终复核，素材 / hitBox / CSS / TS 均无漂移 |
| 2026-11-09 23:40:26 | SCR-011 | 通过（2/2，退出码 0） | `traces/精灵产物校验-20261109-234026.md` | `--check` 复核 10 姿势 + 1 附加帧，693508 B / 100 帧 |
| 2026-11-10 00:06:29 | SCR-014 | **不通过**（3 张产物均未执行外观改动） | `traces/精灵形象候选试片-20261110-000629.md` | Qwen-Image-Edit 4 步 Lightning + 保持优先长提示词，无法产出换发色 / 换裤装；结论：改形象须重出文生图定稿 |
| 2026-11-10 00:07:57 | SCR-013 | **不通过**（3 张姿态与标准站姿几乎一致） | `traces/精灵关键姿态试片-20261110-000757.md` | 转头 / 歪头 / 伸懒腰均未生效；1000 帧真动作素材的内容生产能力本身是硬阻断 |
| 2026-11-10 00:17:43 | SCR-006 | 通过（72/72，退出码 0） | `traces/桌面精灵链路契约检查-20261110-001743.md` | 新增 C41~C45；C03/C15/C21 旧 needle 同步到 v1.2 实现；同期 `npm test` / `npm run build` / `cargo test --lib`（271 通过）/ SCR-010 5/5 / SCR-011 2/2 全绿 |

## 五、报告索引规则

- 报告按版本分目录存放于 `reports/sugt<版本>上线评估/`，命名与编号规则见 `reports/README.md`。
- 报告一旦出具，其编号、结论、版本对应关系必须登记回本表「关联版本报告」列，形成双向可查。
