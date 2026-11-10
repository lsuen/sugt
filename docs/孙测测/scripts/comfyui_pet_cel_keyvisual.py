#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT 桌面精灵：赛璐璐化新造型「唯一正面定稿」候选生成（文生图 / 锁 seed）。

编写人员：孙测测
作用用途：
  以用户 2026-11-10 提供的 3D 半写实造型设定稿为「内容」，以项目赛璐璐
  画风为「形式」，用 Qwen-Image 纯文生图重出唯一正面定稿候选。多个固定
  seed 用于判断链路一致性；产物只落临时目录，绝不写入 ``src/assets/pet``。
  生成后自动做纯白底色键抠图与 105×297 / 96 色「进包适配」对照图，供人工拍板。
适用项目：SUGT 桌面精灵 v1.2 素材链路（本地内网 ComfyUI，非公网服务）
创建日期：2026-11-10
复用方式：
  python3 comfyui_pet_cel_keyvisual.py --out /tmp/sugt-pet-cel-keyvisual
  可选：--seeds 20261110 20261111 20261112 / --width 896 --height 2048
  环境变量 SUGT_COMFYUI_BASE 覆盖服务地址。需先满足：
  curl 可达 ComfyUI（本机 Python socket 被拦，模块内已装 curl 适配器）、
  Qwen-Image fp8 / qwen_2.5_vl_7b / qwen_image_vae / Lightning-8steps 权重齐备。
  后处理对照图需要 Pillow；缺失时跳过对照图但仍出图并留痕。
退出码约定：0 全部成功 / 1 提交或生成失败 / 2 参数、前置检查或解析异常
"""
import argparse
import json
import os
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime
from pathlib import Path

# 办公网代理会把内网 ComfyUI 判成 502；且本机 Python socket 直连被系统拦截。
# 必须在任何请求发起前安装 curl 传输适配器。
sys.path.insert(0, str(Path(__file__).resolve().parent))
import comfyui_curl_http  # noqa: E402

comfyui_curl_http.install()

_ROOT = Path(__file__).resolve().parents[3]
_TRACES = _ROOT / "docs" / "孙测测" / "traces"
_BASE = os.environ.get("SUGT_COMFYUI_BASE", "http://192.168.31.84:8188")
_TIMEOUT_SEC = 420

_UNET = "qwen_image_fp8_e4m3fn.safetensors"
_LORA = "Qwen-Image-Lightning-8steps-V1.0.safetensors"
_CLIP = "qwen_2.5_vl_7b_fp8_scaled.safetensors"
_VAE = "qwen_image_vae.safetensors"

# 造型设定来自用户提供稿（配色 / 骨相 / 机能工装 / 配饰 / 无蓝），
# 画风与信息密度按桌宠 105×297 小窗重写为赛璐璐。
POSITIVE = """solid pure white flat background, uniform white backdrop,
no gradient, no shadow, no floor, no scenery, no vignette。
full body head to toe standing portrait, both legs fully drawn,
both shoes visible, both feet flat on the ground, no cropping,
character centered with 8 percent white margin at top and bottom。
日系动画赛璐璐官方设定稿，纯平涂色块，硬边二段阴影，
均匀深蓝黑线稿，无渐变无喷枪无3D无噪点无投影。
角色：中性清冷少年气质的东方少女，5.5到6头身，小头高颅顶，短鹅蛋脸。
黑紫长发，深紫灰紫挑染，发尾渐变绛红，额前碎刘海，半束高位，后侧长发垂至腰，
发丝只用几缕清晰平涂色块并收束不外扩。
放大版狭长丹凤眼，四层分层虹膜，低饱和灰绿色瞳孔，灰绿不是紫不是蓝，
粗上睫毛，双白高光。浅暖白皙肤色，一小块平涂腮红。
上身：炭灰色宽松机能连帽夹克，帽子自然垂在后背，浅灰结构线、黑色织带、
金属扣环、工装口袋；内搭纯白宽松短袖T恤，胸前低饱和灰紫几何图案，无文字。
下半身必须完整画出：深炭灰色宽松机能工装束脚裤，立体裁片与大工装口袋，
裤脚收束，两条完整小腿，白色中筒棉袜，黑灰白三色厚底复古篮球运动鞋，
双脚平放站稳，双腿之间有清晰间隙。
腰侧黑色斜挎织带挂白紫小兔子挂件；黑色窄皮质颈环带银色圆环扣。
整套服装绝无蓝色，薄荷绿只用于领口滚边小面积点缀。"""

NEGATIVE = """3D, 写实, 照片, PBR, 次表面散射, 厚涂, 渐变, 喷枪, 柔光, 发光滤镜,
景深, 投影, 噪点, 水彩, 材质贴图, 环境光遮蔽, 复杂背景, 图案背景, 棋盘格,
checkerboard, grid, 地面阴影, 天使光环, halo, 光环, 翅膀, wings, 羽毛,
蓝色, blue, 蓝色衣服, 蓝色配饰, 蓝条纹, 拼色条纹毛衣, 高领毛衣, 短发,
紫色眼睛, 蓝色眼睛, purple eyes, blue eyes,
光环发光, 头顶圆环, 天使光圈, 头顶任何圆环或发光物, glowing ring, glowing halo above head, halo, angel ring,
灰蓝渐变背景, 深色背景, 双色背景, 地面投影, 脚底阴影,
呆毛, 文字, 水印, 签名, 多人, 肥胖, 矮壮, 过度幼态Q版, 兽耳, 猫耳,
武器, 多余肢体, 手指畸形, 六指, 四条手臂, 四只手, 重复手臂,
裁切, 半身, 膝上构图, 大腿截断, 缺少小腿, 缺少脚, 脚被裁掉,
cropped at thighs, knee-up framing, missing legs, missing feet,
low resolution, 脏边, 片状头发, 根根分明写实发丝, 大波浪"""


def http_get(path, timeout=30):
    """GET 取回原始字节；走 curl 适配器，绕过本机 socket 限制。"""
    with urllib.request.urlopen(_BASE + path, timeout=timeout) as resp:
        return resp.read()


def http_post_json(path, payload, timeout=60):
    """POST JSON 并解析响应。"""
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        _BASE + path, data=data, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def preflight():
    """校验服务、节点与权重齐备，避免排队后才发现配置缺失。"""
    info = json.loads(http_get("/object_info", timeout=40))
    missing_nodes = [
        name for name in (
            "UNETLoader", "LoraLoaderModelOnly", "CLIPLoader", "VAELoader",
            "CLIPTextEncode", "EmptySD3LatentImage", "KSampler", "VAEDecode",
            "SaveImage",
        ) if name not in info
    ]
    if missing_nodes:
        raise RuntimeError("缺少节点: %s" % ", ".join(missing_nodes))
    weights = {
        "UNETLoader": ("unet_name", _UNET),
        "LoraLoaderModelOnly": ("lora_name", _LORA),
        "CLIPLoader": ("clip_name", _CLIP),
        "VAELoader": ("vae_name", _VAE),
    }
    for node, (field, expected) in weights.items():
        options = info[node]["input"]["required"][field][0]
        if expected not in options:
            raise RuntimeError("%s 缺少权重 %s" % (node, expected))
    clip_types = info["CLIPLoader"]["input"]["required"]["type"][0]
    if "qwen_image" not in clip_types:
        raise RuntimeError("CLIPLoader 不支持 qwen_image 类型")
    stats = json.loads(http_get("/system_stats", timeout=20))
    dev = (stats.get("devices") or [{}])[0]
    print("服务正常 | GPU %s | 空闲显存 %.2f GB" % (
        dev.get("name", "?"), dev.get("vram_free", 0) / 1024 ** 3))
    print("节点与权重前置检查通过")


def build_prompt(seed, width, height):
    """构造 Qwen-Image 纯文生图 API workflow（8 步 Lightning / cfg 1.0）。"""
    return {
        "1": {"class_type": "UNETLoader",
              "inputs": {"unet_name": _UNET, "weight_dtype": "default"}},
        "2": {"class_type": "LoraLoaderModelOnly",
              "inputs": {"model": ["1", 0], "lora_name": _LORA,
                         "strength_model": 1.0}},
        "3": {"class_type": "CLIPLoader",
              "inputs": {"clip_name": _CLIP, "type": "qwen_image",
                         "device": "default"}},
        "4": {"class_type": "VAELoader",
              "inputs": {"vae_name": _VAE}},
        "5": {"class_type": "CLIPTextEncode",
              "inputs": {"text": POSITIVE, "clip": ["3", 0]}},
        "6": {"class_type": "CLIPTextEncode",
              "inputs": {"text": NEGATIVE, "clip": ["3", 0]}},
        "7": {"class_type": "EmptySD3LatentImage",
              "inputs": {"width": width, "height": height, "batch_size": 1}},
        "8": {"class_type": "KSampler",
              "inputs": {"model": ["2", 0], "seed": seed, "steps": 8,
                         "cfg": 1.0, "sampler_name": "euler",
                         "scheduler": "simple", "positive": ["5", 0],
                         "negative": ["6", 0], "latent_image": ["7", 0],
                         "denoise": 1.0}},
        "9": {"class_type": "VAEDecode",
              "inputs": {"samples": ["8", 0], "vae": ["4", 0]}},
        "10": {"class_type": "SaveImage",
               "inputs": {"images": ["9", 0],
                          "filename_prefix": "SUGT/su-cel-keyvisual-%d" % seed}},
    }


def find_history(prompt_id):
    """读取生成历史，兼容单条与全量两种端点。"""
    for path in ("/history/" + prompt_id, "/history?max_items=30"):
        try:
            data = json.loads(http_get(path, timeout=30))
            if prompt_id in data:
                return data[prompt_id]
        except Exception:
            continue
    return None


def generate_one(seed, width, height, out_dir):
    """提交单张、轮询取回，返回 (本地路径, 耗时秒)。"""
    result = http_post_json(
        "/prompt",
        {"prompt": build_prompt(seed, width, height),
         "client_id": "sugt-sdet-cel-keyvisual"},
    )
    if "prompt_id" not in result:
        raise RuntimeError("提交返回异常: %s" % json.dumps(
            result, ensure_ascii=False)[:1200])
    if result.get("node_errors"):
        raise RuntimeError("节点报错: %s" % json.dumps(
            result["node_errors"], ensure_ascii=False)[:1200])
    prompt_id = result["prompt_id"]
    print("  seed=%d 已提交 prompt_id=%s" % (seed, prompt_id))

    start = time.time()
    last_report = 0
    while time.time() - start < _TIMEOUT_SEC:
        hist = find_history(prompt_id)
        if hist:
            status = hist.get("status", {}) or {}
            if status.get("status_str") == "error":
                raise RuntimeError("生成报错: %s" % json.dumps(
                    status, ensure_ascii=False)[:1200])
            if status.get("completed") or hist.get("outputs"):
                images = []
                for _node, node_out in (hist.get("outputs") or {}).items():
                    images.extend(node_out.get("images", []) or [])
                if not images:
                    raise RuntimeError("历史已完成但无图片输出")
                item = images[0]
                query = urllib.parse.urlencode({
                    "filename": item.get("filename", ""),
                    "subfolder": item.get("subfolder", ""),
                    "type": item.get("type", "output"),
                })
                raw = http_get("/view?" + query, timeout=90)
                target = out_dir / ("cel-%d.png" % seed)
                target.write_bytes(raw)
                print("  seed=%d 完成，%.1fs，%.1f KB" % (
                    seed, time.time() - start, len(raw) / 1024))
                return target, time.time() - start
        elapsed = time.time() - start
        if elapsed - last_report >= 30:
            print("  seed=%d 等待中 %.0fs ..." % (seed, elapsed))
            last_report = elapsed
        time.sleep(3)
    raise RuntimeError("超时 %ds，prompt_id=%s" % (_TIMEOUT_SEC, prompt_id))


def build_fit_compare(paths, out_dir):
    """按 contain 105×297 展示造型，并单独输出「背景能否干净抠图」诊断。

    Qwen-Image 常被要求纯白底却给出渐变 / 地面阴影，此时强行色键会把
    整图当成人物（v1/v2 实测 bbox 占全图 100%），产出误导性的假对照图。
    改为直接 contain 展示，把可抠性作为独立可判定诊断输出，不阻断视觉评估。
    """
    try:
        from PIL import Image, ImageDraw, ImageFont
    except ImportError:
        print("Pillow 不可用，跳过进包适配对照图（不影响定稿出图）")
        return None, ["Pillow 不可用，未做背景诊断"]

    target = (105, 297)
    panels = []
    diagnostics = []
    for path in paths:
        im = Image.open(path).convert("RGB")
        w, h = im.size
        corners = [im.getpixel(pt) for pt in (
            (4, 4), (w - 5, 4), (4, h - 5), (w - 5, h - 5))]
        spans = [max(c[k] for c in corners) - min(c[k] for c in corners)
                 for k in range(3)]
        darkest = min(min(c) for c in corners)
        uniform = max(spans) <= 24
        keyable = uniform and darkest > 205
        diagnostics.append(
            "- `%s` 四角 RGB 极差 R/G/B = %d/%d/%d，最暗通道 %d → 背景%s，色键%s" % (
                path.name, spans[0], spans[1], spans[2], darkest,
                "均匀" if uniform else "**不均匀**",
                "可抠" if keyable else "**不可抠（需 AI matting 或重出）**"))
        print("  %s 背景极差 %s 最暗 %d → %s" % (
            path.name, spans, darkest, "可抠" if keyable else "不可抠"))

        fit = im.copy()
        fit.thumbnail(target, Image.LANCZOS)
        base = Image.new("RGB", target, (255, 255, 255))
        base.paste(fit, ((target[0] - fit.width) // 2,
                         (target[1] - fit.height) // 2))
        q96 = base.quantize(colors=96, method=Image.Quantize.MEDIANCUT,
                            dither=Image.Dither.NONE).convert("RGB")
        panels.append((path.stem, base, q96, base.crop((28, 22, 78, 92))))

    scale, bar, gap, pad = 3, 32, 14, 14
    cell_w = target[0] * scale
    cell_h = target[1] * scale
    face_h = 420
    cw = len(panels) * cell_w + gap * (len(panels) - 1) + pad * 2
    ch = cell_h + bar + face_h + bar + pad * 3 + 30
    canvas = Image.new("RGB", (cw, ch), "#141a28")
    draw = ImageDraw.Draw(canvas)
    try:
        font = ImageFont.truetype("/System/Library/Fonts/PingFang.ttc", 16)
        small = ImageFont.truetype("/System/Library/Fonts/PingFang.ttc", 13)
    except OSError:
        font = small = ImageFont.load_default()
    for idx, (stem, plain, quantized, face) in enumerate(panels):
        x = pad + idx * (cell_w + gap)
        draw.rectangle((x, pad, x + cell_w, pad + bar), fill="#1f2b40")
        draw.text((x + 8, pad + 7), stem + " → 105x297 @3x",
                  fill="#e8edf7", font=font)
        canvas.paste(plain.resize((cell_w, cell_h), Image.LANCZOS),
                     (x, pad + bar))
        y2 = pad + bar + cell_h + pad
        draw.rectangle((x, y2, x + cell_w, y2 + bar), fill="#1f2b40")
        draw.text((x + 8, y2 + 7), "脸部 50x70 @6x / 96色",
                  fill="#e8edf7", font=font)
        canvas.paste(face.resize((300, 420), Image.LANCZOS), (x, y2 + bar))
        canvas.paste(quantized.resize((105, 297), Image.LANCZOS),
                     (x + 210, y2 + bar + 40))
    draw.text((pad, ch - 26),
              "下排左：脸部真实像素放大；右：96 色量化后的整图。用于判断眨眼与视线是否可辨。",
              fill="#f5b95c", font=small)
    out = out_dir / "cel-fit-compare.png"
    canvas.save(out)
    return out, diagnostics


def write_trace(out_dir, seeds, width, height, results, compare,
                 diagnostics, started):
    """写 Markdown 执行留痕，含完整 prompt 与参数，便于复现。"""
    _TRACES.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    path = _TRACES / ("精灵赛璐璐定稿候选-%s.md" % stamp)
    lines = [
        "# 精灵赛璐璐化定稿候选生成留痕",
        "",
        "- 执行时间：%s" % started.strftime("%Y-%m-%d %H:%M:%S"),
        "- 脚本：SCR-015 `docs/孙测测/scripts/comfyui_pet_cel_keyvisual.py`",
        "- 服务：`%s`（curl 传输适配器）" % _BASE,
        "- 模型：`%s` + `%s`，8 steps / cfg 1.0 / euler / simple" % (_UNET, _LORA),
        "- 分辨率：`%d×%d`，纯文生图，denoise 1.0" % (width, height),
        "- seed：%s" % ", ".join(str(s) for s in seeds),
        "- 输出目录：`%s`（临时目录，**不进包**）" % out_dir,
        "",
        "## 产物",
        "",
        "| seed | 文件 | 字节 | 耗时 |",
        "| --- | --- | --- | --- |",
    ]
    for seed, path_item, elapsed, size in results:
        lines.append("| %d | `%s` | %d | %.1fs |" % (
            seed, path_item.name, size, elapsed))
    lines += [
        "",
        "- 进包适配对照图：`%s`" % (compare if compare else "未生成（Pillow 不可用或色键失败）"),
        "",
        "## 背景可抠性诊断",
        "",
    ] + diagnostics + [
        "",
        "## 正向提示词",
        "",
        "```text",
        POSITIVE,
        "```",
        "",
        "## 反向提示词",
        "",
        "```text",
        NEGATIVE,
        "```",
        "",
        "## 边界声明",
        "",
        "- 本轮产物仅用于人工选稿，**未写入 `src/assets/pet`**，未替换任何进包素材。",
        "- Qwen / Lightning LoRA 的随包分发许可仍未逐项核实，定稿拍板后须先闭环。",
        "- 色指定表待更新：眼色由 `#7FB4E8` 改为灰绿 `#8FB9A6`，新增薄荷绿 `#5DD9A8` 锚点；",
        "  该项随定稿一并生效，本轮不改动 `docs/design/su-mascot.html`。",
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def main():
    parser = argparse.ArgumentParser(description="赛璐璐化定稿候选（文生图 / 锁 seed）")
    parser.add_argument("--seeds", nargs="+", type=int,
                        default=[20261110, 20261111, 20261112])
    parser.add_argument("--width", type=int, default=1024)
    parser.add_argument("--height", type=int, default=1536)
    parser.add_argument("--out", default=str(
        Path(tempfile.gettempdir()) / "sugt-pet-cel-keyvisual"))
    args = parser.parse_args()

    if args.width < 256 or args.height < 256 or args.width % 8 or args.height % 8:
        print("分辨率必须为 >=256 且为 8 的倍数")
        return 2
    if not args.seeds or any(s < 0 for s in args.seeds):
        print("seed 必须为非负整数列表")
        return 2

    out_dir = Path(args.out).resolve()
    out_dir.mkdir(parents=True, exist_ok=True)
    started = datetime.now()
    print("输出目录:", out_dir)
    print("分辨率 %d×%d | seed %s" % (args.width, args.height, args.seeds))

    # 硬红线：产物只落临时目录，绝不写进包素材目录。
    assets = _ROOT / "src" / "assets" / "pet"
    if out_dir == assets.resolve() or assets.resolve() in out_dir.parents:
        print("拒绝执行：输出目录不得位于 src/assets/pet 内")
        return 2

    try:
        preflight()
    except Exception as exc:
        print("前置检查失败:", exc)
        return 2

    results = []
    try:
        for seed in args.seeds:
            path_item, elapsed = generate_one(
                seed, args.width, args.height, out_dir)
            results.append((seed, path_item, elapsed, path_item.stat().st_size))
    except Exception as exc:
        print("生成失败:", exc)
        write_trace(out_dir, args.seeds, args.width, args.height,
                    results, None, ["生成中断，未做色键诊断"], started)
        return 1

    compare, diagnostics = None, ["对照图未执行"]
    try:
        compare, diagnostics = build_fit_compare(
            [r[1] for r in results], out_dir)
    except Exception as exc:
        print("对照图生成失败（不阻断出图）:", exc)
    trace = write_trace(out_dir, args.seeds, args.width, args.height,
                        results, compare, diagnostics, started)
    print("留痕:", trace)
    print("完成 %d/%d 张" % (len(results), len(args.seeds)))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("已中断")
        sys.exit(1)
    except Exception as exc:
        print("解析/运行异常:", repr(exc))
        sys.exit(2)
