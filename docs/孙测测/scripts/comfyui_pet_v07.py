#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT 小su 桌面精灵 v0.7：本地 ComfyUI 洋红底状态帧 + 带 alpha 动画 WebP。

编写人员：孙测测
作用用途：
  1) states 模式：以洋红纯色底定稿为参考，用 Qwen-Edit 一次产出全部状态差分帧，
     背景保持同色，供落包脚本做确定性色键（发丝缝隙不再依赖泛洪连通性）。
  2) anim 模式：把落包脚本生成的 RGBA 序列帧上传，交给 ComfyUI
     `SaveAnimatedWEBP` 编码成带透明通道的动画 WebP（真实逐帧动画，非图片叠加）。
适用项目：SUGT 桌面精灵素材链路 v0.7
创建日期：2026-11-08
复用方式：
  python3 comfyui_pet_v07.py states <洋红底参考图绝对路径> [state ...]
  python3 comfyui_pet_v07.py anim <序列帧目录> [state ...]
  序列帧目录结构：<dir>/<state>/frame-00.png ... frame-NN.png（RGBA PNG）
  环境变量 SUGT_COMFYUI_BASE 覆盖服务地址，SUGT_PET_OUT 覆盖输出目录
退出码约定：0 全部成功 / 1 提交或生成失败 / 2 参数或解析异常
"""
import json
import os
import struct
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

BASE = os.environ.get("SUGT_COMFYUI_BASE", "http://192.168.31.84:8188")
_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
_DEFAULT_OUT = os.path.join(_ROOT, "docs", "design", "assets", "generated", "v4")
OUT_DIR = os.environ.get("SUGT_PET_OUT", _DEFAULT_OUT)
TIMEOUT_SEC = 1200

UNET_EDIT = "qwen_image_edit_2509_fp8_e4m3fn.safetensors"
LORA_EDIT = "Qwen-Image-Edit-2509-Lightning-4steps-V1.0-bf16.safetensors"
CLIP_NAME = "qwen_2.5_vl_7b_fp8_scaled.safetensors"
VAE_NAME = "qwen_image_vae.safetensors"

# 所有差分帧都强调「背景保持同一块纯净洋红」，否则 Qwen-Edit 会偷偷把底色改掉，
# 色键阈值就会在不同帧之间漂移。
EDIT_BASE = (
    "严格保持这个角色的外观、发型、发色、服装、配色、构图、人物大小、画风完全一致，"
    "只做出以下改变：{change}。"
    "背景必须保持与参考图完全相同的纯净洋红纯色，每一个背景像素都是同一个颜色，"
    "绝对不能有渐变、图案、纹理、网格、地面阴影或杂色，也不要把洋红色染到人物上。"
    "角色始终只有两条手臂和两只手，绝不新增、复制或残留多余的手臂与手；"
    "不要把原本垂在身侧的手臂在新位置再画一遍。"
    "保持正面全身站姿、完整双腿与双脚、赛璐璐硬边平涂、干净均匀线稿，"
    "胸口六边形徽章保持空白无图案，不要添加任何文字、水印或签名。"
)

# idle 直接用洋红底定稿，不经过编辑，避免无谓的画面漂移
EDIT_JOBS = {
    "typing": (
        "保持双臂自然垂在身体两侧完全不动，视线略向下看，表情变得专注，"
        "身后浮现几枚半透明小方块数据粒子；人物大小与位置和参考图完全相同，不要缩放或重新构图",
        20261126,
    ),
    "done": (
        "保持双臂自然垂在身体两侧完全不动，笑容更明显，眼睛明亮，"
        "周围浮现少量六边形光点，人物大小与位置和参考图完全相同",
        20261111,
    ),
    "error": (
        "头明显歪向一侧，眉头皱起，嘴角微微向下，露出困惑表情，"
        "双臂保持自然下垂不动，人物大小与位置和参考图完全相同",
        20261124,
    ),
    "sleep": (
        "把两只眼睛完全闭合：用两道简单的向下弯弧线表示闭眼，"
        "彻底擦除瞳孔、虹膜、眼白和高光，不要画任何圆点、实心圆或橙色圆，"
        "头微低，表情放松安静，人物大小与位置和参考图完全相同",
        20261123,
    ),
    "blink": (
        "只把两只眼睛完全闭合：用两道简单的向下弯弧线表示闭眼，"
        "彻底擦除瞳孔、虹膜、眼白和高光，不要画任何圆点、实心圆或橙色圆；"
        "头部角度、表情、身体姿势、人物大小与位置全部保持与参考图完全相同",
        20261127,
    ),
}

STATES = ("idle", "typing", "done", "error", "sleep")


def http_get(path, timeout=30):
    with urllib.request.urlopen(BASE + path, timeout=timeout) as resp:
        return resp.read()


def http_post_json(path, payload, timeout=60):
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        BASE + path, data=data, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def upload_image(path, tag=None):
    """把本地 RGBA PNG 上传到 ComfyUI input，返回 LoadImage 可用的文件名。"""
    boundary = "----sugt" + uuid.uuid4().hex
    with open(path, "rb") as handle:
        raw = handle.read()
    filename = tag or os.path.basename(path)
    body = (
        ("--%s\r\nContent-Disposition: form-data; name=\"image\"; filename=\"%s\"\r\n"
         "Content-Type: image/png\r\n\r\n" % (boundary, filename)).encode()
        + raw + b"\r\n"
        + ("--%s\r\nContent-Disposition: form-data; name=\"overwrite\"\r\n\r\ntrue\r\n"
           % boundary).encode()
        + ("--%s--\r\n" % boundary).encode()
    )
    req = urllib.request.Request(
        BASE + "/upload/image", data=body,
        headers={"Content-Type": "multipart/form-data; boundary=" + boundary},
    )
    with urllib.request.urlopen(req, timeout=180) as resp:
        info = json.loads(resp.read().decode("utf-8"))
    name = info.get("name") or filename
    sub = info.get("subfolder") or ""
    return os.path.join(sub, name).replace("\\", "/") if sub else name


def build_states(ref_name, states):
    """一次提交产出全部差分帧：共享模型加载，避免反复换入显存。"""
    prompt = {
        "1": {
            "class_type": "UNETLoader",
            "inputs": {"unet_name": UNET_EDIT, "weight_dtype": "default"},
        },
        "2": {
            "class_type": "LoraLoaderModelOnly",
            "inputs": {"model": ["1", 0], "lora_name": LORA_EDIT, "strength_model": 1.0},
        },
        "3": {
            "class_type": "ModelSamplingAuraFlow",
            "inputs": {"model": ["2", 0], "shift": 3.0},
        },
        "4": {
            "class_type": "CLIPLoader",
            "inputs": {"clip_name": CLIP_NAME, "type": "qwen_image", "device": "default"},
        },
        "5": {"class_type": "VAELoader", "inputs": {"vae_name": VAE_NAME}},
        "6": {"class_type": "LoadImage", "inputs": {"image": ref_name}},
        "7": {"class_type": "VAEEncode", "inputs": {"pixels": ["6", 0], "vae": ["5", 0]}},
        "8": {"class_type": "CLIPTextEncode", "inputs": {"text": "", "clip": ["4", 0]}},
    }
    for index, state in enumerate(states):
        change, seed = EDIT_JOBS[state]
        base = 20 + index * 4
        prompt[str(base)] = {
            "class_type": "TextEncodeQwenImageEdit",
            "inputs": {
                "clip": ["4", 0], "prompt": EDIT_BASE.format(change=change),
                "vae": ["5", 0], "image": ["6", 0],
            },
        }
        prompt[str(base + 1)] = {
            "class_type": "KSampler",
            "inputs": {
                "model": ["3", 0], "seed": seed, "steps": 4, "cfg": 1.0,
                "sampler_name": "euler", "scheduler": "simple",
                "positive": [str(base), 0], "negative": ["8", 0],
                "latent_image": ["7", 0], "denoise": 1.0,
            },
        }
        prompt[str(base + 2)] = {
            "class_type": "VAEDecode",
            "inputs": {"samples": [str(base + 1), 0], "vae": ["5", 0]},
        }
        prompt[str(base + 3)] = {
            "class_type": "SaveImage",
            "inputs": {
                "images": [str(base + 2), 0],
                "filename_prefix": "SUGT/v7-chroma-%s" % state,
            },
        }
    return prompt


def build_anim(frames_by_state):
    """把每个状态的 RGBA 序列帧串成 batch，交给 ComfyUI 编码动画 WebP。

    `SaveAnimatedWEBP` 的 lossless=False 仍保留 alpha（WebP 的 ALPH 块独立压缩），
    体积比无损小一个数量级；fps=8 与落包脚本生成的 8 帧一一对应。
    """
    prompt = {}
    node_id = 100
    for state, names in frames_by_state.items():
        loaded = []
        for name in names:
            prompt[str(node_id)] = {"class_type": "LoadImage", "inputs": {"image": name}}
            loader = str(node_id)
            node_id += 1
            # LoadImage 会把 RGBA 拆成 IMAGE(RGB) + MASK(alpha)，
            # 不还原就会把透明通道丢掉，动画 WebP 变成洋红/黑底。
            prompt[str(node_id)] = {
                "class_type": "JoinImageWithAlpha",
                "inputs": {"image": [loader, 0], "alpha": [loader, 1]},
            }
            loaded.append([str(node_id), 0])
            node_id += 1
        batch = loaded[0]
        for item in loaded[1:]:
            prompt[str(node_id)] = {
                "class_type": "ImageBatch",
                "inputs": {"image1": batch, "image2": item},
            }
            batch = [str(node_id), 0]
            node_id += 1
        prompt[str(node_id)] = {
            "class_type": "SaveAnimatedWEBP",
            "inputs": {
                "images": batch,
                "filename_prefix": "SUGT/v7-loop-%s" % state,
                "fps": 8.0,
                "lossless": False,
                "quality": 88,
                "method": "slowest",
            },
        }
        node_id += 1
    return prompt


def submit_and_fetch(prompt, label, rename=None):
    """提交任务、轮询历史、取回全部图片产物。"""
    os.makedirs(OUT_DIR, exist_ok=True)
    print("模式:", label, "| 目标:", BASE, "| 输出:", OUT_DIR, flush=True)
    try:
        result = http_post_json(
            "/prompt", {"prompt": prompt, "client_id": "sugt-pet-v07"}
        )
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", "replace")
        print("提交失败 HTTP %s: %s" % (exc.code, body[:3000]), flush=True)
        return 1
    except Exception as exc:
        print("提交失败:", exc, flush=True)
        return 1
    if "prompt_id" not in result:
        print("提交返回异常:", json.dumps(result, ensure_ascii=False)[:3000], flush=True)
        return 1
    prompt_id = result["prompt_id"]
    print("已提交 prompt_id =", prompt_id, flush=True)
    if result.get("node_errors"):
        print("节点警告:", json.dumps(result["node_errors"], ensure_ascii=False)[:2500], flush=True)

    start = time.time()
    last_report = 0
    while time.time() - start < TIMEOUT_SEC:
        try:
            hist = json.loads(http_get("/history/" + prompt_id))
        except Exception:
            hist = {}
        rec = hist.get(prompt_id)
        if rec:
            status = rec.get("status", {}) or {}
            if status.get("status_str") == "error":
                print("生成报错:", json.dumps(status, ensure_ascii=False)[:3000], flush=True)
                return 1
            if status.get("completed") or rec.get("outputs"):
                print("生成完成，耗时 %.1fs" % (time.time() - start), flush=True)
                images = []
                for _node, out in (rec.get("outputs") or {}).items():
                    for item in out.get("images", []) or []:
                        images.append(item)
                if not images:
                    print("历史已完成但未找到图片输出", flush=True)
                    return 1
                for item in images:
                    query = urllib.parse.urlencode({
                        "filename": item.get("filename", ""),
                        "subfolder": item.get("subfolder", ""),
                        "type": item.get("type", "output"),
                    })
                    raw = http_get("/view?" + query, timeout=120)
                    filename = item.get("filename", "output.png")
                    if rename:
                        filename = rename(filename) or filename
                    target = os.path.join(OUT_DIR, filename)
                    with open(target, "wb") as handle:
                        handle.write(raw)
                    print("已取回: %s (%.1f KB)" % (target, len(raw) / 1024), flush=True)
                return 0
        elapsed = time.time() - start
        if elapsed - last_report >= 20:
            print("  等待中 %.0fs ..." % elapsed, flush=True)
            last_report = elapsed
        time.sleep(3)
    print("超时 %ds 未完成，prompt_id=%s" % (TIMEOUT_SEC, prompt_id), flush=True)
    return 1


def webp_chunks(path):
    """解析 WebP 的 RIFF 块，返回 (块列表, VP8X 标志位)。"""
    data = open(path, "rb").read()
    chunks = []
    flags = 0
    offset = 12
    while offset + 8 <= len(data):
        tag = data[offset:offset + 4]
        size = struct.unpack("<I", data[offset + 4:offset + 8])[0]
        chunks.append((tag, size))
        if tag == b"VP8X" and size >= 1:
            flags = data[offset + 8]
        offset += 8 + size + (size & 1)
    return chunks, flags


def finalize_loops():
    """清掉 ComfyUI 写入的 EXIF，并校验动画帧数与 alpha 位。

    VP8X 标志：0x10 = alpha，0x08 = EXIF，0x02 = animation。
    """
    results = []
    for name in sorted(os.listdir(OUT_DIR)):
        if not (name.startswith("su-") and name.endswith("-loop.webp")):
            continue
        path = os.path.join(OUT_DIR, name)
        chunks, flags = webp_chunks(path)
        # 只在确实带 EXIF 时清理；重复执行时 webpmux 会因「无 EXIF」报错，属正常。
        if any(tag == b"EXIF" for tag, _size in chunks):
            clean = path + ".clean"
            try:
                subprocess.run(
                    ["webpmux", "-strip", "exif", path, "-o", clean],
                    check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                )
                os.replace(clean, path)
            except (OSError, subprocess.CalledProcessError):
                if os.path.exists(clean):
                    os.remove(clean)
                print("警告：EXIF 清理失败，保留原文件:", name, flush=True)
            chunks, flags = webp_chunks(path)
        frames = sum(1 for tag, _size in chunks if tag == b"ANMF")
        has_alpha = bool(flags & 0x10)
        has_anim = bool(flags & 0x02) or frames > 0
        ok = frames >= 2 and has_alpha and has_anim
        results.append((name, frames, has_alpha, os.path.getsize(path) / 1024, ok))
        print("校验 %-24s %d 帧 alpha=%s 动画=%s %.2f KB %s" % (
            name, frames, has_alpha, has_anim, os.path.getsize(path) / 1024,
            "OK" if ok else "不通过"), flush=True)
    return all(row[4] for row in results) and bool(results)


def main():
    if len(sys.argv) < 3 or sys.argv[1] not in ("states", "anim"):
        print(__doc__)
        return 2
    mode, source = sys.argv[1], sys.argv[2]
    if mode == "states":
        if not os.path.isfile(source):
            print("洋红底参考图不存在:", source)
            return 2
        states = sys.argv[3:] or [s for s in STATES if s != "idle"]
        unknown = [s for s in states if s not in EDIT_JOBS]
        if unknown:
            print("未知状态:", unknown, "| 可选:", list(EDIT_JOBS))
            return 2
        ref = upload_image(source, "v7-chroma-base.png")
        print("已上传参考图:", ref, flush=True)
        return submit_and_fetch(build_states(ref, states), "states=%s" % states)
    # anim 模式
    if not os.path.isdir(source):
        print("序列帧目录不存在:", source)
        return 2
    wanted = sys.argv[3:] or list(STATES)
    frames_by_state = {}
    for state in wanted:
        folder = os.path.join(source, state)
        names = sorted(
            n for n in os.listdir(folder)
            if n.lower().endswith(".png") and n.startswith("frame-")
        ) if os.path.isdir(folder) else []
        if not names:
            print("缺少序列帧:", state, folder)
            return 2
        uploaded = []
        for index, name in enumerate(names):
            tag = "v7-%s-%02d.png" % (state, index)
            uploaded.append(upload_image(os.path.join(folder, name), tag))
        frames_by_state[state] = uploaded
        print("已上传 %-7s %d 帧" % (state, len(uploaded)), flush=True)

    def rename(filename):
        for state in wanted:
            if "v7-loop-%s" % state in filename:
                return "su-%s-loop.webp" % state
        return None

    result = submit_and_fetch(build_anim(frames_by_state), "anim=%s" % wanted, rename)
    if result != 0:
        return result
    return 0 if finalize_loops() else 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("已中断")
        sys.exit(1)
    except Exception as exc:
        print("解析/运行异常:", repr(exc))
        sys.exit(2)
