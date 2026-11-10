#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SCR-019：桌面精灵 v1.3 动作注册表 1000 帧素材批量生产线。

编写人员：孙测测
作用用途：
  从用户手抠定稿 good.png 出发，按 src/assets/pet/manifest.json 的 actions
  注册表逐动作生产专属序列帧：主图回填纯青蓝底 → Qwen-Image-Edit 2509 派生
  关键姿态 → MiniMax H3 FL2VA 补间 → 逐帧自适应色键抠图 → 统一坐标系归一化
  到 manifest.baseSize（180×297）源帧 → 交 pet_reencode_assets.py 按
  manifest.encoding.colors（56 色）做 lossless 整套重编码；色数与画布均从
  manifest 读取，禁止在脚本里写死第二份真相源。
适用项目：SUGT 桌面精灵 v1.3（内网 ComfyUI 192.168.31.84，非公网服务）
创建日期：2026-11-10
复用方式：
  python3 comfyui_pet_action_series.py --phase prepare
  python3 comfyui_pet_action_series.py --phase all            # 断点续跑，可反复执行
  python3 comfyui_pet_action_series.py --phase all --action lifted   # 单动作验证
  python3 comfyui_pet_action_series.py --phase encode        # 只做归一化落包
  SUGT_PET_WORK=/tmp/sugt-pet-actions-v13 python3 comfyui_pet_action_series.py \
      --phase frames --recut                                   # 只用原始视频离线重建
  --phase frames 完全离线（只读本地 videos/*.webp），不调用 ComfyUI；逐帧抠图
  结果缓存在 <work>/cutouts/<动作>/*.png，二次调整归一化参数时秒级复跑，
  只有 --recut 才会重新抠图。原始视频即唯一图源，任何尺寸/留白问题都靠缩放与
  平移解决，绝不重新生成、绝不裁掉动作中的肢体。
  环境变量 SUGT_COMFYUI_BASE 覆盖服务地址；SUGT_PET_WORK 覆盖工作目录。
  本机 Python socket 直连被拦，模块内安装 curl 传输适配器；后处理需 Pillow
  （本机 /usr/local/bin/python3.13）。
退出码约定：0 全部成功 / 1 生成或验收失败 / 2 参数、前置或解析异常

红线遵循：
  - 唯一真相源是 manifest.json 的 poses[] 动作注册表，脚本不另存动作清单；
  - 素材落地前先过 SCR-018 序列验收，不得只换静态海报；
  - 所有动作共用同一 bbox 坐标系，切换动作不跳位；
  - 许可由项目所有者 2026-11-10 拍板放行，见 ASSETS.md 第六节。
"""
import argparse
import json
import math
import os
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from datetime import datetime
from pathlib import Path

_SCRIPTS = Path(__file__).resolve().parent
sys.path.insert(0, str(_SCRIPTS))
import comfyui_curl_http  # noqa: E402

comfyui_curl_http.install()

_ROOT = _SCRIPTS.parents[2]
_TRACES = _ROOT / "docs" / "孙测测" / "traces"
_MANIFEST = _ROOT / "src" / "assets" / "pet" / "manifest.json"
_ASSETS = _ROOT / "src" / "assets" / "pet"
_BASE = os.environ.get("SUGT_COMFYUI_BASE", "http://192.168.31.84:8188")
_WORK = Path(os.environ.get("SUGT_PET_WORK", "/tmp/sugt-pet-actions"))
_TIMEOUT = 2400

# 背景色来自用户定稿 20261110013708_8_5.jpg 的实测纯青蓝底。
_BACKDROP = (91, 145, 190)
# 底部/四周纯色留白：由代码填充，绝不要求模型「画留白」，避免亮青椭圆平台。
_FOOT_MARGIN = 0.12
# v1.3.3：512 宽度不够——H3 补间会拉近人物，双臂平举 / 挥手 / 飘发时
# 手指与发梢被画布左右边缘永久裁掉（用户可见「睡醒丢手」）。
# 加宽到 640（16 的倍数，H3 latent 约束），主体缩小 + 四边门禁一起兜底；
# 落包画布仍是 manifest.baseSize，多余分辨率只在离线归一化阶段消化。
# 允许用环境变量覆盖做提速验证，但必须与 master/关键帧保持同一画布。
_CANVAS = tuple(
    int(part) for part in os.environ.get("SUGT_PET_CANVAS", "640x768").split("x")
)
_H3_STEPS = int(os.environ.get("SUGT_PET_H3_STEPS", "8"))
_EDIT_STEPS = int(os.environ.get("SUGT_PET_EDIT_STEPS", "4"))

_UNET_EDIT = "qwen_image_edit_2509_fp8_e4m3fn.safetensors"
_LORA_LIGHTNING = "Qwen-Image-Edit-2509-Lightning-4steps-V1.0-bf16.safetensors"
_LORA_ANGLES = "Qwen-Edit-2509-Multiple-angles.safetensors"
_CLIP_EDIT = "qwen_2.5_vl_7b_fp8_scaled.safetensors"
_VAE_EDIT = "qwen_image_vae.safetensors"

_UNET_H3 = "minimax_h3_fl2va_pruned_int8_convrot.safetensors"
_LORA_H3 = "minimax_h3_fl2v_turbo_8step_v1.0_comfyui_bf16.safetensors"
_CLIP_H3 = "qwen3vl_32b_minimax_h3_nvfp4_awq.safetensors"
_VAE_H3 = "minimax_h3_video_vae_fp16.safetensors"

_IDENTITY = (
    "Keep this exact character: black-purple long hair with crimson tips, "
    "purple eyes, grey hoodie over white T-shirt, black cargo joggers, "
    "white socks and sneakers, white rabbit charm, mint hair clip. "
    "Cel-shaded anime flat colours, hard two-tone shadows, clean dark outlines. "
)
_UPRIGHT = (
    "The character stays UPRIGHT at all times: head at the top, feet at the bottom, "
    "same body orientation as the reference, never rotated or turned over. "
)
_HARD = (
    _UPRIGHT +
    "solid pure cyan-blue flat background, uniform backdrop everywhere, "
    "no gradient, no scenery, no floor, no ground shadow, no platform, "
    "full body head to toe, both legs fully drawn, both shoes visible, "
    "BOTH ARMS AND BOTH HANDS COMPLETELY INSIDE THE FRAME, "
    "wide empty cyan-blue margin on all four sides of the character, "
    "even with arms stretched sideways the fingertips stay far away from "
    "the left and right image borders, nothing is cut off by any edge, "
    "large empty cyan-blue area below the shoes of the SAME flat colour, "
    "centred character, identical framing and scale as the reference. "
)
_INVERTED = (
    "upside down, inverted, flipped vertically, rotated 180 degrees, "
    "hanging by the feet, head at the bottom, feet at the top, lying down"
)
_NEG_EDIT = (
    "3D, photorealistic, PBR, gradient background, white background, floor shadow, "
    "changed outfit, changed hair colour, short hair, blue clothing, "
    "bright cyan ellipse, glowing platform, podium, spotlight circle, ground disc, "
    "turquoise circle, stage, halo, angel ring, wings, watermark, text, "
    "cropped feet, missing shoes, knee-up framing, extra limbs, deformed hands, "
    "cropped hands, cropped arms, limbs cut off by the image border, "
    "fingertips touching the left or right edge, character too large for the frame, "
    + _INVERTED
)
_NEG_H3 = (
    "3D, photorealistic, gradient background, white background, floor shadow, "
    "changed outfit, changed hair colour, identity drift, face morph, "
    "bright cyan ellipse, glowing platform, podium, spotlight circle, "
    "cropped feet, missing shoes, extra limbs, deformed hands, watermark, text, "
    "cropped hands, cropped arms, limbs cut off by the image border, "
    "fingertips touching the left or right edge, character too large for the frame, "
    + _INVERTED
)

# 每个动作的姿态描述：loop 动作用同一张关键姿态做首尾（闭环），
# once 动作给起止两张。文案保持简短，硬约束由 _HARD 统一前置。
_POSE_PROMPTS = {
    "lifted": ("gently lifted a few centimetres into the air by an unseen hand, "
               "STILL STANDING UPRIGHT with head at the top and feet at the bottom, "
               "both feet just off the ground and dangling, arms raised a little, "
               "hair and legs hanging down and swaying slightly"),
    "landing": ("standing upright after being put down, knees slightly bent "
                "from landing, hair settling back"),
    "typing": ("looking down at a small keyboard, both hands typing, "
               "slight forward lean"),
    "error": ("startled short pause, shoulders tensed, eyes looking down"),
    "done": ("standing tall with a small satisfied lift of the chin"),
    "sleep": ("eyes closed, head tilted, relaxed shoulders, dozing while standing"),
    "offline": ("quiet standby, arms relaxed at sides, calm neutral expression"),
    "celebrate": ("both arms thrown up in a happy jump, feet leaving the ground"),
    "disappointed": ("head bowed, shoulders drooping, eyes cast down"),
    "petted": ("being patted on the head, shoulders hunched shyly, slight blush"),
    "poked": ("one hand raised in a small acknowledging gesture, head tilted"),
    "wake": ("stretching both arms up and yawning after waking"),
    "welcome": ("one hand raised waving in greeting, friendly open posture"),
    "greeting": ("small polite nod of the head"),
    "wait": ("glancing left and right while waiting, hands held in front"),
    "stretch": ("big lazy stretch, arms overhead, back arched slightly"),
    "lookAround": ("head turning to look around, curious expression"),
    "peek": ("leaning forward and peeking, head craned slightly"),
    "walk": ("mid-stride walking pose, one leg forward, arms swinging naturally"),
    "idle": ("neutral relaxed standing pose, gentle breathing, arms at sides"),
}

# once 动作的结束姿态（起点用 _POSE_PROMPTS，终点用这里）
_END_PROMPTS = {
    "landing": "fully settled standing pose, knees straight, hair still",
    "wake": "awake neutral standing pose, arms lowered",
    "welcome": "neutral standing pose, waving hand lowered",
    "greeting": "neutral standing pose, head back to centre",
    "stretch": "neutral standing pose, arms back at sides",
    "lookAround": "neutral standing pose, head facing forward",
}


def http_get(path, timeout=60):
    with urllib.request.urlopen(_BASE + path, timeout=timeout) as resp:
        return resp.read()


def http_post_json(path, payload, timeout=60):
    req = urllib.request.Request(
        _BASE + path, data=json.dumps(payload).encode("utf-8"),
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def upload_image(path, name=None):
    """上传本地图到 ComfyUI input，返回可在 LoadImage 里引用的文件名。"""
    import uuid
    filename = name or ("sugt-%s.png" % uuid.uuid4().hex[:10])
    boundary = "----sugtBoundary" + uuid.uuid4().hex
    body = bytearray()
    for field, value in (("overwrite", "true"),):
        body += ("--%s\r\nContent-Disposition: form-data; name=\"%s\"\r\n\r\n%s\r\n"
                 % (boundary, field, value)).encode()
    body += ("--%s\r\nContent-Disposition: form-data; name=\"image\"; "
             "filename=\"%s\"\r\nContent-Type: image/png\r\n\r\n"
             % (boundary, filename)).encode()
    body += Path(path).read_bytes()
    body += ("\r\n--%s--\r\n" % boundary).encode()
    req = urllib.request.Request(
        _BASE + "/upload/image", data=bytes(body),
        headers={"Content-Type": "multipart/form-data; boundary=" + boundary},
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        result = json.loads(resp.read().decode("utf-8"))
    return result.get("name", filename)


def submit_and_fetch(prompt, tag, out_file, timeout=_TIMEOUT):
    """提交 workflow，轮询历史，取回唯一输出文件。返回 (路径, 耗时秒)。"""
    result = http_post_json(
        "/prompt", {"prompt": prompt, "client_id": "sugt-sdet-actions"})
    if "prompt_id" not in result:
        raise RuntimeError("提交返回异常: %s" % json.dumps(
            result, ensure_ascii=False)[:1200])
    if result.get("node_errors"):
        raise RuntimeError("节点报错: %s" % json.dumps(
            result["node_errors"], ensure_ascii=False)[:1200])
    pid = result["prompt_id"]
    print("  [%s] 已提交 %s" % (tag, pid[:12]), flush=True)
    start = time.time()
    last = 0.0
    while time.time() - start < timeout:
        try:
            hist = json.loads(http_get("/history/" + pid, timeout=30))
            item = hist.get(pid)
            if item:
                status = item.get("status", {}) or {}
                if status.get("status_str") == "error":
                    raise RuntimeError("生成报错: %s" % json.dumps(
                        status, ensure_ascii=False)[:1200])
                images = []
                for _node, node in (item.get("outputs") or {}).items():
                    images.extend(node.get("images", []) or [])
                if images:
                    meta = images[0]
                    query = urllib.parse.urlencode({
                        "filename": meta.get("filename", ""),
                        "subfolder": meta.get("subfolder", ""),
                        "type": meta.get("type", "output")})
                    raw = http_get("/view?" + query, timeout=300)
                    out_file.parent.mkdir(parents=True, exist_ok=True)
                    out_file.write_bytes(raw)
                    print("  [%s] 完成 %.1fs / %.1f KB" % (
                        tag, time.time() - start, len(raw) / 1024), flush=True)
                    return out_file, time.time() - start
        except RuntimeError:
            raise
        except Exception as exc:
            if time.time() - last > 90:
                print("  [%s] 轮询异常(忽略): %s" % (tag, exc), flush=True)
                last = time.time()
        elapsed = time.time() - start
        if elapsed - last >= 60:
            print("  [%s] 等待 %.0fs ..." % (tag, elapsed), flush=True)
            last = elapsed
        time.sleep(5)
    raise RuntimeError("超时 %ds，prompt_id=%s" % (timeout, pid))


def build_edit_prompt(reference, pose_text, seed):
    return {
        "1": {"class_type": "UNETLoader", "inputs": {
            "unet_name": _UNET_EDIT, "weight_dtype": "default"}},
        "2": {"class_type": "LoraLoaderModelOnly", "inputs": {
            "model": ["1", 0], "lora_name": _LORA_LIGHTNING,
            "strength_model": 1.0}},
        "3": {"class_type": "LoraLoaderModelOnly", "inputs": {
            "model": ["2", 0], "lora_name": _LORA_ANGLES,
            "strength_model": 0.85}},
        "4": {"class_type": "CLIPLoader", "inputs": {
            "clip_name": _CLIP_EDIT, "type": "qwen_image",
            "device": "default"}},
        "5": {"class_type": "VAELoader", "inputs": {"vae_name": _VAE_EDIT}},
        "6": {"class_type": "LoadImage", "inputs": {"image": reference}},
        "7": {"class_type": "TextEncodeQwenImageEditPlus", "inputs": {
            "clip": ["4", 0], "vae": ["5", 0], "image1": ["6", 0],
            "prompt": _HARD + _IDENTITY + "Action: " + pose_text + "."}},
        "8": {"class_type": "CLIPTextEncode", "inputs": {
            "clip": ["4", 0], "text": _NEG_EDIT}},
        "9": {"class_type": "EmptySD3LatentImage", "inputs": {
            "width": _CANVAS[0], "height": _CANVAS[1], "batch_size": 1}},
        "10": {"class_type": "KSampler", "inputs": {
            "model": ["3", 0], "seed": seed, "steps": _EDIT_STEPS, "cfg": 1.0,
            "sampler_name": "euler", "scheduler": "simple",
            "positive": ["7", 0], "negative": ["8", 0],
            "latent_image": ["9", 0], "denoise": 1.0}},
        "11": {"class_type": "VAEDecode", "inputs": {
            "samples": ["10", 0], "vae": ["5", 0]}},
        "12": {"class_type": "SaveImage", "inputs": {
            "images": ["11", 0], "filename_prefix": "SUGT/su-action-key"}},
    }


def build_h3_prompt(first_frame, last_frame, pose_text, length, seed,
                    playback="loop"):
    tail = ("a smooth cyclic motion that returns to the starting pose"
            if playback == "loop" else
            "a smooth one-way transition from the first pose to the last pose")
    motion = (", clear visible movement of the arms, hair and body between frames, "
              "not a static hold, " + tail)
    return {
        "1": {"class_type": "UNETLoader", "inputs": {
            "unet_name": _UNET_H3, "weight_dtype": "default"}},
        "2": {"class_type": "LoraLoaderModelOnly", "inputs": {
            "model": ["1", 0], "lora_name": _LORA_H3, "strength_model": 1.0}},
        "3": {"class_type": "MiniMaxH3SigmaShift", "inputs": {
            "model": ["2", 0], "shift_video": 12.0, "shift_audio": 3.0}},
        "4": {"class_type": "CLIPLoader", "inputs": {
            "clip_name": _CLIP_H3, "type": "minimax", "device": "default"}},
        "5": {"class_type": "VAELoader", "inputs": {"vae_name": _VAE_H3}},
        "6": {"class_type": "LoadImage", "inputs": {"image": first_frame}},
        "7": {"class_type": "LoadImage", "inputs": {"image": last_frame}},
        "8": {"class_type": "MiniMaxH3ImageToVideo", "inputs": {
            "clip": ["4", 0], "vae": ["5", 0],
            "prompt": _HARD + _IDENTITY + "Motion: " + pose_text + motion,
            "width": _CANVAS[0], "height": _CANVAS[1], "length": length,
            "first_frame": ["6", 0], "last_frame": ["7", 0]}},
        "9": {"class_type": "CLIPTextEncode", "inputs": {
            "clip": ["4", 0], "text": _NEG_H3}},
        "10": {"class_type": "KSampler", "inputs": {
            "model": ["3", 0], "seed": seed, "steps": _H3_STEPS, "cfg": 1.0,
            "sampler_name": "euler", "scheduler": "simple",
            "positive": ["8", 0], "negative": ["9", 0],
            "latent_image": ["8", 1], "denoise": 1.0}},
        "11": {"class_type": "VAEDecode", "inputs": {
            "samples": ["10", 0], "vae": ["5", 0]}},
        "12": {"class_type": "SaveAnimatedWEBP", "inputs": {
            "images": ["11", 0],
            "filename_prefix": "SUGT/su-action-video",
            "fps": 24.0, "lossless": True, "quality": 80,
            "method": "default"}},
    }


def preflight():
    info = json.loads(http_get("/object_info", timeout=60))
    required = ("UNETLoader", "LoraLoaderModelOnly", "CLIPLoader", "VAELoader",
                "CLIPTextEncode", "TextEncodeQwenImageEditPlus",
                "EmptySD3LatentImage", "KSampler", "VAEDecode", "SaveImage",
                "LoadImage", "MiniMaxH3SigmaShift", "MiniMaxH3ImageToVideo",
                "SaveAnimatedWEBP")
    missing = [name for name in required if name not in info]
    if missing:
        raise RuntimeError("缺少节点: %s" % ", ".join(missing))
    weights = {
        "UNETLoader": ("unet_name", (_UNET_EDIT, _UNET_H3)),
        "LoraLoaderModelOnly": ("lora_name", (_LORA_LIGHTNING, _LORA_ANGLES, _LORA_H3)),
        "CLIPLoader": ("clip_name", (_CLIP_EDIT, _CLIP_H3)),
        "VAELoader": ("vae_name", (_VAE_EDIT, _VAE_H3)),
    }
    for node, (field, expected) in weights.items():
        options = info[node]["input"]["required"][field][0]
        absent = [value for value in expected if value not in options]
        if absent:
            raise RuntimeError("%s 缺少权重 %s" % (node, absent))
    stats = json.loads(http_get("/system_stats", timeout=20))
    dev = (stats.get("devices") or [{}])[0]
    print("前置检查通过 | %s | 空闲显存 %.2f GB" % (
        dev.get("name", "?"), dev.get("vram_free", 0) / 1024 ** 3), flush=True)


def load_state():
    path = _WORK / "state.json"
    if path.is_file():
        return json.loads(path.read_text(encoding="utf-8"))
    return {"master": None, "keys": {}, "videos": {}, "frames": {},
            "timings": {}, "encoded": False}


def save_state(state):
    _WORK.mkdir(parents=True, exist_ok=True)
    (_WORK / "state.json").write_text(
        json.dumps(state, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8")


def prepare_master(state, args):
    """把用户手抠定稿合成回纯青蓝底，并用代码填出脚部留白。"""
    from PIL import Image
    if state.get("master") and not args.force:
        return state["master"]
    source = _ROOT / "docs" / "孙测测" / "traces" / "assets" / "su-cel-good-fixed.png"
    if not source.is_file():
        raise RuntimeError("缺定稿 %s（先跑 SCR-017 --fix）" % source)
    char = Image.open(source).convert("RGBA")
    usable_h = int(_CANVAS[1] * (1 - _FOOT_MARGIN))
    # v1.3.3：0.94 会让「双臂平举 / 挥手 / 探头」的关键帧与 H3 补间把手指、
    # 头发顶到 512 画布边缘，肢体像素在源视频里就永久缺失（用户可见「丢手」）。
    # 缩小主体占比，四边留出足够青蓝空白；归一化按 tight_box 等量放大，
    # 落包人物尺寸不变，但手脚不会再被源画布裁掉。
    master_scale = float(os.environ.get("SUGT_PET_MASTER_SCALE", "0.78"))
    scale = min(_CANVAS[0] / char.width, usable_h / char.height) * master_scale
    size = (int(char.width * scale), int(char.height * scale))
    char = char.resize(size, Image.LANCZOS)
    canvas = Image.new("RGBA", _CANVAS, _BACKDROP + (255,))
    canvas.paste(char, ((_CANVAS[0] - size[0]) // 2,
                        int(_CANVAS[1] * 0.02)), char)
    master_path = _WORK / "su-master.png"
    master_path.parent.mkdir(parents=True, exist_ok=True)
    canvas.convert("RGB").save(master_path)
    state["master"] = upload_image(master_path, "sugt-action-master.png")
    save_state(state)
    print("主图就绪 | %s | 画布 %s | 脚部留白 %.0f%%" % (
        state["master"], _CANVAS, _FOOT_MARGIN * 100), flush=True)
    return state["master"]


def do_keys(state, actions, args):
    master = state["master"]
    for index, action in enumerate(actions):
        name = action["name"]
        if name in state["keys"] and not args.force:
            continue
        prompt_text = _POSE_PROMPTS.get(name)
        if not prompt_text:
            raise RuntimeError("动作 %s 缺姿态提示词（扩展动作须同步登记）" % name)
        once = action["playback"] == "once"
        seed = 20261200 + index * 17
        start_file = _WORK / "keys" / ("%s-start.png" % name)
        start_ref_name = "sugt-key-%s-start.png" % name
        guarded_edit(master, prompt_text, seed, "%s/start" % name, start_file)
        start_ref = upload_image(start_file, start_ref_name)

        if once:
            end_file = _WORK / "keys" / ("%s-end.png" % name)
            end_text = _END_PROMPTS.get(name, "neutral standing pose")
            # 终点从起点派生，保证造型/构图连续。
            guarded_edit(start_ref, end_text, seed + 1, "%s/end" % name, end_file)
            end_ref = upload_image(end_file, "sugt-key-%s-end.png" % name)
        else:
            # loop 动作首尾同源，H3 生成一段自然闭环，播放无硬跳。
            end_ref = start_ref
        state["keys"][name] = {"first": start_ref, "last": end_ref,
                              "once": once}
        save_state(state)
    print("关键姿态阶段完成 %d/%d" % (
        len(state["keys"]), len(actions)), flush=True)


def do_interpolate(state, actions, args):
    # 长批次（20 段 × 约 2 分钟）不能因为一段顽固触边就整批中断：
    # 三段都不过就记入 rejected 并继续，末尾统一抛错，凭 state.json 断点续跑。
    rejected = state.setdefault("rejected", [])
    for index, action in enumerate(actions):
        name = action["name"]
        if name in state["videos"] and not args.force:
            continue
        keys = state["keys"].get(name)
        if not keys:
            raise RuntimeError("动作 %s 缺关键姿态，先跑 --phase keys" % name)
        video = _WORK / "videos" / ("%s.webp" % name)
        prompt_text = _POSE_PROMPTS[name]
        # 补间同样要过四边门禁：关键帧不触边不代表 H3 中途不会把手臂
        # 甩出画布。源视频一旦裁掉，后续抠图/归一化都补不回像素。
        detail = ""
        for attempt in range(3):
            seed = 20261300 + index * 23 + attempt * 101
            _, elapsed = submit_and_fetch(
                build_h3_prompt(keys["first"], keys["last"], prompt_text,
                                args.length, seed, action["playback"]),
                "%s/h3-%d%s" % (name, args.length,
                                "/retry%d" % attempt if attempt else ""), video)
            sides, _worst, detail = edge_touch_score(video)
            print("  [%s] 补间四边门禁 %s | %s" % (
                name, "通过" if sides == 0 else "不通过", detail), flush=True)
            if sides == 0:
                if name in rejected:
                    rejected.remove(name)
                break
            if attempt < 2:
                print("  [%s] 肢体触到源画布边缘，换 seed 重生成" % name,
                      flush=True)
        else:
            if name not in rejected:
                rejected.append(name)
            print("  [%s] 三段补间均触边，标记待返工，继续其余动作" % name,
                  flush=True)
            save_state(state)
            continue
        state["videos"][name] = str(video)
        state["timings"][name] = round(elapsed, 1)
        save_state(state)
    print("补间阶段完成 %d/%d" % (
        len(state["videos"]), len(actions)), flush=True)
    if rejected:
        raise RuntimeError("四边门禁未通过，需单独返工：%s"
                           % ", ".join(rejected))


def orientation_score(source):
    """朝向门禁（SCR-019）：白色鞋袜质心必须在肤色脸/手质心之下。

    实测定稿正立图为 0.464 - 0.248 = +0.216；把「被提起来」理解成倒挂的
    废片为 0.307 - 0.389 = -0.082。差值 > 0.08 且白质心 > 0.38 才放行，
    避免 180° 倒置的关键姿态混进 1000 帧（语义不符，且统一 bbox 会跳位）。
    """
    from PIL import Image
    image = Image.open(source).convert("RGB") if not hasattr(source, "load") \
        else source.convert("RGB")
    width, height = image.size
    pixels = image.load()
    skin_y = skin_n = white_y = white_n = 0.0
    for y in range(0, height, 3):
        for x in range(0, width, 3):
            r, g, b = pixels[x, y]
            if r > 170 and g > 110 and b > 90 and r > g + 12 and r - b > 40:
                skin_y += y
                skin_n += 1
            if r > 195 and g > 195 and b > 195:
                white_y += y
                white_n += 1
    if skin_n < 50 or white_n < 50:
        return False, 0.0, "肤色/白色像素过少，无法判定朝向"
    skin = skin_y / skin_n / height
    white = white_y / white_n / height
    score = white - skin
    detail = "肤色质心 %.3f / 白色质心 %.3f / 差值 %+.3f" % (skin, white, score)
    return score > 0.08 and white > 0.38, score, detail


def guarded_edit(reference, pose_text, seed, tag, out_file):
    """生成关键姿态并过「朝向 + 四边不触边」双门禁；不通过换 seed 重试一次。"""
    last = ""
    for attempt in range(3):
        submit_and_fetch(
            build_edit_prompt(reference, pose_text, seed + attempt * 7),
            "%s%s" % (tag, "/retry" if attempt else ""), out_file)
        upright, _score, detail = orientation_score(out_file)
        sides, _worst, edge_detail = edge_touch_score(out_file)
        print("  [%s] 朝向 %s | 四边 %s | %s / %s" % (
            tag, "通过" if upright else "不通过",
            "通过" if sides == 0 else "不通过", detail, edge_detail), flush=True)
        if upright and sides == 0:
            return out_file
        last = "%s / %s" % (detail, edge_detail)
        if attempt < 2:
            print("  [%s] 门禁不通过，换 seed 重试" % tag, flush=True)
    raise RuntimeError("%s 连续三次生成门禁不通过，拒绝进入补间：%s"
                       % (tag, last))


def tight_box(alpha):
    """用行列投影抑制零星残留像素，取人物主体的稳健包围盒。

    H3 逐帧背景会有 1~2px 色度噪点，直接 getbbox() 会把 bbox 拉到画布四边；
    实测 20 段中有 157 个抽样帧触边，但 alpha 占比始终 0.31~0.53（色键没坏）。
    阈值取该列/行采样点数的 1.5%，人物主体远超，孤立噪点被排除。
    """
    width, height = alpha.size
    pixels = alpha.load()
    step = 2
    columns = [0] * width
    rows = [0] * height
    for y in range(0, height, step):
        for x in range(0, width, step):
            if pixels[x, y] > 8:
                columns[x] += 1
                rows[y] += 1
    col_min = max(4, int(height / step * 0.015))
    row_min = max(4, int(width / step * 0.015))
    xs = [x for x, count in enumerate(columns) if count >= col_min]
    ys = [y for y, count in enumerate(rows) if count >= row_min]
    if not xs or not ys:
        return alpha.getbbox()
    return (min(xs), min(ys), max(xs) + 1, max(ys) + 1)


def _quantile(values, fraction):
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, int(len(ordered) * fraction)))
    return ordered[index]


def _corner_background(image):
    """逐帧四角采样背景色，绝不硬编码单一色值（Edit/H3 都会漂移）。"""
    width, height = image.size
    points = ((4, 4), (width - 5, 4), (4, height - 5), (width - 5, height - 5))
    samples = [image.convert("RGB").getpixel(point) for point in points]
    return tuple(sum(sample[channel] for sample in samples) // len(samples)
                 for channel in range(3))


def edge_touch_score(path, band_ratio=0.025, sample_step=2, limit=60):
    """生成期硬门禁：色键之前检查「非背景实体」是否贴到画布四边。

    v1.3.2 的教训：源视频 512 画布边缘已经把 wake / stretch 的手指、
    landing 的飘发永久裁掉，后面抠图与归一化再完美也补不回像素。
    因此门禁必须放在生成侧、用原始青蓝底帧判定，而不是等落包才验。
    返回 (触边方向数, 四边最大抽样实体数, 详情)。
    """
    from PIL import Image
    image = Image.open(path)
    count = getattr(image, "n_frames", 1)
    indices = (sorted({round(i * (count - 1) / 11) for i in range(12)})
               if count > 1 else [0])
    band_x = max(4, round(image.width * band_ratio))
    band_y = max(4, round(image.height * band_ratio))
    worst = [0, 0, 0, 0]
    hit_frames = []
    for index in indices:
        image.seek(index)
        rgb = image.convert("RGB")
        width, height = rgb.size
        background = _corner_background(rgb)
        pixels = rgb.load()
        current = [0, 0, 0, 0]
        for y in range(0, height, sample_step):
            edge_y = y < band_y or y >= height - band_y
            for x in range(0, width, sample_step):
                if not edge_y and not (x < band_x or x >= width - band_x):
                    continue
                red, green, blue = pixels[x, y]
                distance = (abs(red - background[0])
                            + abs(green - background[1])
                            + abs(blue - background[2]))
                # 两道过滤：色差小的是背景漂移；蓝偏（b-r>=45）的是青蓝底
                # 渐变/色度噪点。角色像素（黑发、灰卫衣、肤色、白鞋、红发）
                # 都不满足蓝偏特征，因此只统计真正的肢体触边。
                if distance < 90 or blue - red >= 45:
                    continue
                if x < band_x:
                    current[0] += 1
                if x >= width - band_x:
                    current[1] += 1
                if y < band_y:
                    current[2] += 1
                if y >= height - band_y:
                    current[3] += 1
        worst = [max(old, new) for old, new in zip(worst, current)]
        if max(current) > limit:
            hit_frames.append(index)
    sides = sum(1 for value in worst if value > limit)
    detail = "四边实体 L/R/T/B=%s | 触边帧 %s" % (worst, hit_frames[:8])
    return sides, max(worst), detail


def _cut_one(job):
    """子进程：单动作从原始补间视频逐帧抠图，写 PNG 缓存，只回传统计。

    不回传图像对象（跨进程序列化 1000 张 512×768 RGBA 会吃满内存），
    主进程随后按帧逐个读取缓存，峰值内存恒定在单动作的几十 MB。
    """
    name, video, wanted, cache_root, recut = job
    from pathlib import Path
    from PIL import Image
    sys.path.insert(0, str(_SCRIPTS))
    import check_pet_keyvisual_cutout as key_module

    cache = Path(cache_root) / name
    cached = sorted(cache.glob("*.png")) if cache.is_dir() else []
    if len(cached) == wanted and not recut:
        return (name, len(cached), "复用缓存", "原始视频未改动")
    im = Image.open(video)
    count = getattr(im, "n_frames", 1)
    if count < wanted:
        raise RuntimeError("%s 仅 %d 帧，不足 %d" % (name, count, wanted))
    im.seek(0)
    upright, _score, detail = orientation_score(im.convert("RGB"))
    if not upright:
        raise RuntimeError("%s 补间序列朝向门禁不通过：%s" % (name, detail))
    indices = [round(i * (count - 1) / (wanted - 1)) for i in range(wanted)]
    cache.mkdir(parents=True, exist_ok=True)
    last_background = "未知"
    for index, frame_index in enumerate(indices):
        im.seek(frame_index)
        rgb = im.convert("RGB")
        background = _corner_background(rgb)
        last_background = str(background)
        key_module.chroma_key(rgb, background).save(
            cache / ("%03d.png" % index), format="PNG")
    return (name, len(indices), last_background,
            "离线抠图并缓存 | 朝向通过 %s" % detail)


def do_frames(state, actions, args):
    """原始视频 → 抠图（可缓存）→ 纯缩放/平移归一化到 manifest.baseSize。

    v1.3.2 红线：整个过程**不裁人物**。旧版用「代表框 ± 90 分位外扩」做
    crop，伸展动作的手臂落在框外就被永久切掉；新版只做整帧缩放 + 平移，
    角色四边用「该动作 50 帧 bbox 并集」求安全框，并全局解一个所有动作共用
    的目标身高 h_ref，保证 20 动作人物同高、脚底同线、水平居中、四边留白。
    抠图结果缓存在 <work>/cutouts/，参数微调无需重跑昂贵的逐帧色键。
    """
    from PIL import Image, ImageChops

    source_dir = _WORK / "source"
    cutout_dir = _WORK / "cutouts"
    source_dir.mkdir(parents=True, exist_ok=True)
    cutout_dir.mkdir(parents=True, exist_ok=True)
    wanted = int(args.frames)
    names = list(state["videos"].keys())
    # 阶段 A：离线逐帧抠图，结果落 PNG 缓存。逐帧色键是纯 Python 逐像素循环，
    # 单帧约 1.9s，20×50 帧串行约 30 分钟；用进程池并行 + 永久缓存把「以后
    # 调人物尺寸 / 留白」的复跑成本压到分钟级，且完全不碰 ComfyUI。
    from concurrent.futures import ProcessPoolExecutor, as_completed
    jobs = [(name, str(state["videos"][name]), wanted,
             str(cutout_dir), bool(args.recut)) for name in names]
    with ProcessPoolExecutor(max_workers=min(6, len(jobs))) as pool:
        futures = [pool.submit(_cut_one, job) for job in jobs]
        for future in as_completed(futures):
            name, count, background, detail = future.result()
            print("  [%s] %s | 背景 %s | %s" % (
                name, count, background, detail), flush=True)
    cache_paths = {}
    for name in names:
        paths = sorted((cutout_dir / name).glob("*.png"))
        if len(paths) != wanted:
            raise RuntimeError("%s 抠图缓存 %d 帧，不足 %d" % (
                name, len(paths), wanted))
        cache_paths[name] = paths

    # ---- v1.3.2 归一化：并集安全框 + 全局目标身高，全程不裁人物 ----
    # 旧版两套算法都留了缺陷：
    #   v1.3   用「全 1000 帧 bbox 中位数 + 20px」做统一 crop，伸展动作手臂
    #          直接被裁；Qwen-Edit / H3 每段把人画得不一样大，极差 27.9%。
    #   v1.3.1 用「逐动作代表框 ± 90 分位外扩（再钳 25%/15%）」做 crop，
    #          尺度虽归一，但 90 分位与钳制仍可能把极端帧的手脚排除在框外。
    # 新版只做「整帧缩放 + 平移」：每动作取 50 帧稳健 bbox 的**并集**作安全框
    # （并集由定义覆盖每一帧的全部肢体，不做任何分位裁剪），并集外只清色键
    # 噪点；再全局解一个所有动作共用的目标身高 h_ref：
    #   s_i = h_ref / 该动作代表框高度 → 20 动作代表身高恒等于 h_ref；
    #   h_ref 取「最坏动作的四向约束」最小值 → 并集框必然落在画布安全边内。
    # 锚点统一「脚底同线 + 水平居中」，动作内 50 帧共用同一变换，绝不抖动。
    rep, union, ext = {}, {}, {}
    for name in names:
        boxes = []
        for path in cache_paths[name]:
            with Image.open(path) as frame:
                boxes.append(tight_box(frame.getchannel("A")))
        boxes = [box for box in boxes if box]
        if not boxes:
            raise RuntimeError("%s 全部帧无有效 alpha bbox" % name)
        rep[name] = tuple(_quantile([box[i] for box in boxes], 0.5)
                          for i in range(4))
        union[name] = (min(box[0] for box in boxes),
                       min(box[1] for box in boxes),
                       max(box[2] for box in boxes),
                       max(box[3] for box in boxes))
        left, top, right, bottom = rep[name]
        ul, ut, ur, ub = union[name]
        # 补 12 源像素透明边，容纳 LANCZOS 插值外溢；同时计入全局约束。
        pad = 12
        ext[name] = (left - ul + pad, top - ut + pad,
                     ur - right + pad, ub - bottom + pad)

    manifest = json.loads(_MANIFEST.read_text(encoding="utf-8"))
    target = (manifest["baseSize"]["w"], manifest["baseSize"]["h"])
    # 素材四边安全边：与 CSS --pet-inset-* 叠成两层余量，容纳 lifted -10px、
    # hop -7px、breathe/poke 放大，以及窗口 overflow:hidden 的硬裁剪。
    safe_x = max(8, round(target[0] * 0.06))
    safe_y = max(8, round(target[1] * 0.06))
    avail_x, avail_y = target[0] / 2 - safe_x, target[1] / 2 - safe_y
    bounds = []
    for name in rep:
        left, top, right, bottom = rep[name]
        core_w, core_h = right - left, bottom - top
        ext_l, ext_t, ext_r, ext_b = ext[name]
        # 代表框中心贴画布中心、脚底贴脚线，四向并集外扩不得越出安全边。
        for label, value in (
            ("左", avail_x * core_h / (core_w / 2 + ext_l)),
            ("右", avail_x * core_h / (core_w / 2 + ext_r)),
            ("上", avail_y / (0.5 + ext_t / core_h)),
            ("下", avail_y / (0.5 + ext_b / core_h)),
        ):
            bounds.append((value, name, label))
    h_ref = min(item[0] for item in bounds)
    print("  [约束瓶颈] %s" % " / ".join(
        "%s-%s=%.1f" % (name, label, value)
        for value, name, label in sorted(bounds)[:5]), flush=True)
    scale = {name: h_ref / (rep[name][3] - rep[name][1]) for name in rep}
    # 脚线：人物代表高度上下居中，而非沉底。
    foot_y = round((target[1] + h_ref) / 2)
    offset = {}
    for name in rep:
        left, _top, right, bottom = rep[name]
        factor = scale[name]
        offset[name] = (round(target[0] / 2 - (left + right) / 2 * factor),
                        round(foot_y - bottom * factor))
    print("  [归一化] 画布 %dx%d | 安全边 %d/%d px | 目标身高 h_ref=%.1f px"
          " | 脚线 y=%d | 每动作独立 scale（不裁一像素人物）" % (
              target[0], target[1], safe_x, safe_y, h_ref, foot_y), flush=True)

    def hidden_ratio(frame, name):
        """并集安全框外的实体像素占比：>2% 说明 tight_box 漏掉了肢体，必须停。"""
        ul, ut, ur, ub = union[name]
        pad = 12
        safe_box = (max(0, ul - pad), max(0, ut - pad),
                    min(frame.width, ur + pad), min(frame.height, ub + pad))
        solid = frame.getchannel("A").point(
            lambda value: 255 if value > 16 else 0)
        total = solid.histogram()[255]
        if not total:
            return 0.0
        outside = solid.copy()
        outside.paste(0, safe_box)
        return outside.histogram()[255] / total

    def normalise(frame, name):
        """整帧缩放 + 平移；并集框外只清色键噪点，人物像素一个不裁。"""
        ul, ut, ur, ub = union[name]
        pad = 12
        safe_box = (max(0, ul - pad), max(0, ut - pad),
                    min(frame.width, ur + pad), min(frame.height, ub + pad))
        cleaned = frame.copy()
        keep = Image.new("L", frame.size, 0)
        keep.paste(255, safe_box)
        cleaned.putalpha(ImageChops.multiply(cleaned.getchannel("A"), keep))
        factor = scale[name]
        size = (max(1, round(frame.width * factor)),
                max(1, round(frame.height * factor)))
        resized = cleaned.resize(size, Image.LANCZOS)
        fitted = Image.new("RGBA", target, (0, 0, 0, 0))
        fitted.paste(resized, offset[name], resized)
        return fitted

    def edge_of(fitted):
        """归一后角色到画布四边的距离，用于生产线自检。"""
        alpha = fitted.getchannel("A").point(lambda value: 255 if value > 16 else 0)
        box = alpha.getbbox()
        if box is None:
            return (fitted.height, fitted.width, fitted.height, fitted.width)
        return (box[1], fitted.width - box[2],
                fitted.height - box[3], box[0])

    min_edge = None
    max_hidden = 0.0
    action_heights = {}
    per_action = {}
    for name in names:
        normalised = []
        heights = []
        worst_edge = None
        worst_hidden = 0.0
        for path in cache_paths[name]:
            with Image.open(path) as source_frame:
                frame = source_frame.convert("RGBA")
            worst_hidden = max(worst_hidden, hidden_ratio(frame, name))
            fitted = normalise(frame, name)
            edge = edge_of(fitted)
            current = min(edge)
            worst_edge = current if worst_edge is None else min(worst_edge, current)
            min_edge = current if min_edge is None else min(min_edge, current)
            alpha_box = fitted.getchannel("A").point(
                lambda value: 255 if value > 16 else 0).getbbox()
            heights.append(alpha_box[3] - alpha_box[1] if alpha_box else 0)
            normalised.append(fitted)
        max_hidden = max(max_hidden, worst_hidden)
        ordered = sorted(heights)
        action_heights[name] = ordered[len(ordered) // 2]
        per_action[name] = {
            "scale": round(scale[name], 4), "edge": worst_edge,
            "hidden": round(worst_hidden, 5),
            "union": list(union[name]), "ext": list(ext[name]),
        }
        print("    %-14s scale=%.4f 最小留白=%2dpx 框外实体=%.3f%%" % (
            name, scale[name], worst_edge, worst_hidden * 100), flush=True)
        loop_path = source_dir / ("su-%s-loop.webp" % name)
        normalised[0].save(loop_path, format="WEBP", save_all=True,
                           append_images=normalised[1:], duration=100,
                           loop=0, lossless=True, method=4)
        poster_path = source_dir / ("su-%s.webp" % name)
        normalised[0].save(poster_path, format="WEBP",
                           lossless=True, method=4)
        state["frames"][name] = wanted
    # 附加眨眼帧：从 sleep 序列中段取闭眼帧，避免混入旧造型的 su-blink。
    # 不能取第 0 帧：sleep poster 也是第 0 帧，两者字节相同会被构建器按内容
    # 哈希去重，dist 少一张素材且门禁 D04 判红；眨眼与睡姿起始帧亦不应同图。
    blink_paths = cache_paths.get("sleep")
    if blink_paths:
        with Image.open(blink_paths[len(blink_paths) // 2]) as source_frame:
            blink = normalise(source_frame.convert("RGBA"), "sleep")
        blink.save(source_dir / "su-blink.webp", format="WEBP",
                   lossless=True, method=4)
    low, high = min(action_heights.values()), max(action_heights.values())
    spread = high / low - 1 if low else float("inf")
    print("  [归一化自检] 最小四边留白 %d px | 人物高中位 %d~%d px | "
          "极差 %.1f%% | 并集框外实体像素最大 %.3f%%" % (
              min_edge, low, high, spread * 100, max_hidden * 100), flush=True)
    if min_edge < 6 or spread > 0.08 or max_hidden > 0.02:
        raise RuntimeError(
            "归一化未达标：最小四边留白 %d px（需 ≥6）/ 高度极差 %.1f%%（需 ≤8%%）"
            " / 并集框外实体像素 %.3f%%（需 ≤2%%，防止裁掉肢体）"
            % (min_edge, spread * 100, max_hidden * 100))
    state["normalise"] = {
        "algorithm": "v1.3.2 并集安全框 + 全局目标身高（纯缩放，不裁人物）",
        "canvas": list(target), "h_ref": round(h_ref, 2),
        "safe": [safe_x, safe_y], "foot_y": foot_y,
        "min_edge": min_edge, "height_spread": round(spread, 4),
        "max_hidden": round(max_hidden, 5),
        "heights": action_heights, "per_action": per_action,
    }
    save_state(state)
    print("源帧就绪 | %d 动作 × %d 帧 = %d 帧 | %s" % (
        len(names), wanted, sum(state["frames"].values()),
        source_dir), flush=True)
    return source_dir


def do_encode(state, args):
    """统一坐标系后 cropBox 退化为整幅，交给既有重编码脚本整套落包。"""
    data = json.loads(_MANIFEST.read_text(encoding="utf-8"))
    data["cropBox"] = [0, 0, data["baseSize"]["w"], data["baseSize"]["h"]]
    size = (data["baseSize"]["w"], data["baseSize"]["h"])
    data["cropNote"] = (
        "v1.3.2：SCR-019 直接用原始补间视频离线重建，按「逐动作 50 帧 bbox "
        "并集安全框 + 全局目标身高」做纯缩放/平移归一化到 %d×%d；20 动作人物"
        "同高、脚底同线、水平居中、四边留透明安全边，全程不裁任何一像素人物"
        "（并集框外实体像素占比 ≤2%% 为硬门禁）；源帧本身即目标画布，"
        "cropBox 为整幅，切换动作不跳位。" % size)
    _MANIFEST.write_text(
        json.dumps(data, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8")
    source_dir = _WORK / "source"
    cmd = [sys.executable, str(_SCRIPTS / "pet_reencode_assets.py"),
           "--source", str(source_dir)]
    print("落包: %s" % " ".join(cmd), flush=True)
    completed = subprocess.run(cmd, cwd=str(_ROOT), capture_output=True,
                               text=True, timeout=1200)
    print(completed.stdout[-4000:], flush=True)
    if completed.returncode != 0:
        print(completed.stderr[-3000:], file=sys.stderr, flush=True)
        raise RuntimeError("pet_reencode_assets.py 退出码 %d" % completed.returncode)
    state["encoded"] = True
    save_state(state)
    total = sum(path.stat().st_size for path in _ASSETS.glob("*.webp"))
    print("整套素材落包 | %d 张 | %.2f MB" % (
        len(list(_ASSETS.glob("*.webp"))), total / 1024 ** 2), flush=True)


def write_trace(state, actions, args, started):
    _TRACES.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    path = _TRACES / ("精灵动作序列生产-%s.md" % stamp)
    total_frames = sum(state["frames"].values())
    manifest = json.loads(_MANIFEST.read_text(encoding="utf-8"))
    target = (manifest["baseSize"]["w"], manifest["baseSize"]["h"])
    colors = manifest["encoding"]["colors"]
    total_bytes = sum(item.stat().st_size for item in _ASSETS.glob("*.webp"))
    lines = [
        "# 精灵 v1.3 动作序列生产留痕（SCR-019）", "",
        "- 执行时间：%s" % started.strftime("%Y-%m-%d %H:%M:%S"),
        "- 服务：`%s`（curl 传输适配器）" % _BASE,
        "- 工作目录：`%s`" % _WORK,
        "- 定稿：`su-cel-good-fixed.png`（SCR-017 修补版）",
        "- 生成画布：`%dx%d`（目标素材 %d×%d；横向加宽专用于容纳伸展肢体，见四边门禁）" % (_CANVAS + target),
        "- 主图占比：`SUGT_PET_MASTER_SCALE`（默认 0.78），避免人物被 H3 补间拉近后顶到画布边缘",
        "- 关键姿态：Qwen-Image-Edit 2509 + Lightning-4steps + Multiple-angles(0.85) / %d steps" % _EDIT_STEPS,
        "- 补间：MiniMax H3 FL2VA + turbo-8step / %d steps / length=%d" % (_H3_STEPS, args.length),
        "- 朝向门禁：白鞋质心 − 肤色质心 > 0.08（拦 180° 倒挂），关键姿态与补间首帧双重校验",
        "- 四边门禁：`edge_touch_score` 抽 12 帧检查角色像素是否贴到四边 4% 带；关键姿态与补间各允许换 seed 重试一次，仍触边即中断。v1.3.3 起因：512 宽把手指/发梢/鞋底永久裁掉，后处理补不回像素",
        "- 图源：`videos/*.webp` 原始补间视频（v1.3.3 全量重生成）；抠图缓存 `cutouts/`",
        "- 抠图：逐帧四角自适应 `chroma_key`；归一化：并集安全框 + 全局目标身高（纯缩放不裁人物）",
        "- 归一化实测：`%s`"
        % json.dumps({key: value for key, value in state.get("normalise", {}).items()
                      if key != "per_action"}, ensure_ascii=False, sort_keys=True),
        "- 编码：%d 色 palette-lossless / method=%d / %d×%d" % (
            colors, manifest["encoding"]["method"], target[0], target[1]),
        "- 落包体积：`%d B / %.3f MB`（%d 张 WebP）" % (
            total_bytes, total_bytes / 1024 ** 2, len(list(_ASSETS.glob("*.webp")))),
        "- 许可：项目所有者 2026-11-10 拍板放行本地素材进包分发",
        "",
        "## 动作与帧数", "",
        "| 动作 | 语义 | playback | 帧数 | H3 耗时(s) |",
        "| --- | --- | --- | --- | --- |",
    ]
    for action in actions:
        name = action["name"]
        lines.append("| `%s` | %s | %s | %s | %s |" % (
            name, action["semantic"], action["playback"],
            state["frames"].get(name, "-"), state["timings"].get(name, "-")))
    lines += [
        "", "**合计 %d 帧**（独立位图，frameOrder 无重复）" % total_frames,
        "", "## 结论", "",
        "- 关键姿态 %d/%d、补间 %d/%d、源帧 %d/%d" % (
            len(state["keys"]), len(actions), len(state["videos"]),
            len(actions), len(state["frames"]), len(actions)),
        "- 整套落包：%s" % ("已完成" if state.get("encoded") else "未完成"),
        "- 本留痕不替代 SCR-018 序列验收与实窗人工验收。",
    ]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return path


def parse_args():
    parser = argparse.ArgumentParser(description="精灵 v1.3 动作序列批量生产")
    parser.add_argument("--phase", default="all",
                        choices=("prepare", "keys", "interpolate", "frames",
                                 "encode", "all"))
    parser.add_argument("--action", action="append", default=[],
                        help="只处理指定动作，可重复；用于单动作验证与断点续跑")
    parser.add_argument("--length", type=int, default=56,
                        help="H3 单次帧数，须落在 17k+5 栅格")
    parser.add_argument("--frames", type=int, default=50,
                        help="每动作落包帧数（均匀抽取）")
    parser.add_argument("--force", action="store_true", help="忽略断点，强制重跑")
    parser.add_argument("--recut", action="store_true",
                        help="忽略 cutouts 缓存，从原始视频重新逐帧抠图（离线，不调 ComfyUI）")
    parser.add_argument("--out", type=Path, default=None, help="留痕路径")
    return parser.parse_args()


def main():
    args = parse_args()
    global _WORK
    if os.environ.get("SUGT_PET_WORK"):
        _WORK = Path(os.environ["SUGT_PET_WORK"])
    if args.length % 17 != 5:
        raise ValueError("H3 length 必须落在 17k+5 栅格（5/22/39/56/73/90/...）")
    manifest = json.loads(_MANIFEST.read_text(encoding="utf-8"))
    actions = manifest["poses"]
    if args.action:
        unknown = [name for name in args.action
                   if name not in {item["name"] for item in actions}]
        if unknown:
            raise ValueError("未知动作: %s" % unknown)
        actions = [item for item in actions if item["name"] in args.action]
    started = datetime.now()
    state = load_state()
    print("SCR-019 | phase=%s | 动作 %d | length=%d | frames=%d" % (
        args.phase, len(actions), args.length, args.frames), flush=True)

    if args.phase in ("prepare", "all"):
        preflight()
        prepare_master(state, args)
    if args.phase in ("keys", "all"):
        if not state.get("master"):
            raise RuntimeError("缺主图，先跑 --phase prepare")
        do_keys(state, actions, args)
    if args.phase in ("interpolate", "all"):
        do_interpolate(state, actions, args)
    if args.phase in ("frames", "all"):
        do_frames(state, actions, args)
    if args.phase in ("encode", "all"):
        do_encode(state, args)
    trace = write_trace(state, actions, args, started)
    print("留痕 %s" % trace, flush=True)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, json.JSONDecodeError, KeyError) as exc:
        print("前置/解析异常: %r" % exc, file=sys.stderr, flush=True)
        sys.exit(2)
    except RuntimeError as exc:
        print("生成或验收失败: %s" % exc, file=sys.stderr, flush=True)
        sys.exit(1)
