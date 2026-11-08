#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT 小su 素材链路冒烟：向本地 ComfyUI 提交 Qwen-Image 文生图任务。

编写人员：孙测测
作用用途：验证本地 ComfyUI 能否产出全身赛璐璐定稿，记录显存/耗时/产物
适用项目：SUGT 桌面精灵素材链路
创建日期：2026-11-08
复用方式：python3 docs/孙测测/scripts/comfyui_qwen_smoke.py [width] [height] [seed]
          环境变量 SUGT_COMFYUI_BASE 覆盖服务地址，SUGT_SMOKE_OUT 覆盖输出目录
退出码约定：0 生成成功并取回 / 1 生成失败或超时 / 2 读取解析失败
"""
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

# 服务地址与输出目录均可通过环境变量覆盖，不绑定某一台机器的绝对路径
BASE = os.environ.get("SUGT_COMFYUI_BASE", "http://192.168.31.84:8188")
_DEFAULT_OUT = os.path.abspath(
    os.path.join(os.path.dirname(__file__), "..", "..", "design", "assets", "smoke")
)
OUT_DIR = os.environ.get("SUGT_SMOKE_OUT", _DEFAULT_OUT)
TIMEOUT_SEC = 300

WIDTH = int(sys.argv[1]) if len(sys.argv) > 1 else 1024
HEIGHT = int(sys.argv[2]) if len(sys.argv) > 2 else 1536
SEED = int(sys.argv[3]) if len(sys.argv) > 3 else 20261108

POSITIVE = (
    "日系动画赛璐璐立绘，官方设定稿精度。硬边二段阴影、纯平涂色块、"
    "均匀深蓝黑线稿、无渐变无喷枪无3D无噪点。发顶一道月牙形 Angel-ring 高光；"
    "眼睛四层（深虹膜环、雾蓝中调、亮色下弦、双白高光）加粗上睫毛。"
    "角色小su：中性纤细体型，全身站姿，雾蓝色短波波头，发梢卷成信号波纹，"
    "头顶一撮S形呆毛、尖端一个小薄荷光点；雾蓝色眼睛，浅暖肤色，浅笑，头微侧。"
    "米白色立领短外套加深蓝黑拼接，细青色滚边；"
    "胸口正中一枚带细金属框的正六边形核心徽章。"
    "身后两条半透明浅蓝数据流飘带，末端小方块像素点，少量六边形粒子。"
    "正面全身，角色完整居中、脚部不被裁切，留8%安全边距，"
    "纯白干净背景，竖版构图，高清线稿。"
)
NEGATIVE = (
    "3D, 写实, 照片, 厚涂, 渐变, 柔光, 发光滤镜, 景深, 投影, 噪点, 水彩, "
    "复杂背景, 文字, 水印, 签名, 多人, 肥胖, 矮壮, 过度幼态Q版, 兽耳, 猫耳, "
    "翅膀, 武器, 多余肢体, 手指畸形, 六指, 裁切, 半身, 膝上构图, 低分辨率, 脏边"
)


def http_get(path, timeout=20):
    with urllib.request.urlopen(BASE + path, timeout=timeout) as resp:
        return resp.read()


def http_post_json(path, payload, timeout=30):
    data = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(
        BASE + path, data=data, headers={"Content-Type": "application/json"}
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        return json.loads(resp.read().decode("utf-8"))


def build_prompt():
    """构造 ComfyUI API 格式的 Qwen-Image 文生图工作流。"""
    return {
        "1": {
            "class_type": "UNETLoader",
            "inputs": {
                "unet_name": "qwen_image_fp8_e4m3fn.safetensors",
                "weight_dtype": "default",
            },
        },
        "2": {
            "class_type": "LoraLoaderModelOnly",
            "inputs": {
                "model": ["1", 0],
                "lora_name": "Qwen-Image-Lightning-8steps-V1.0.safetensors",
                "strength_model": 1.0,
            },
        },
        "3": {
            "class_type": "CLIPLoader",
            "inputs": {
                "clip_name": "qwen_2.5_vl_7b_fp8_scaled.safetensors",
                "type": "qwen_image",
                "device": "default",
            },
        },
        "4": {
            "class_type": "VAELoader",
            "inputs": {"vae_name": "qwen_image_vae.safetensors"},
        },
        "5": {
            "class_type": "CLIPTextEncode",
            "inputs": {"text": POSITIVE, "clip": ["3", 0]},
        },
        "6": {
            "class_type": "CLIPTextEncode",
            "inputs": {"text": NEGATIVE, "clip": ["3", 0]},
        },
        "7": {
            "class_type": "EmptySD3LatentImage",
            "inputs": {"width": WIDTH, "height": HEIGHT, "batch_size": 1},
        },
        "8": {
            "class_type": "KSampler",
            "inputs": {
                "model": ["2", 0],
                "seed": SEED,
                "steps": 8,
                "cfg": 1.0,
                "sampler_name": "euler",
                "scheduler": "simple",
                "positive": ["5", 0],
                "negative": ["6", 0],
                "latent_image": ["7", 0],
                "denoise": 1.0,
            },
        },
        "9": {
            "class_type": "VAEDecode",
            "inputs": {"samples": ["8", 0], "vae": ["4", 0]},
        },
        "10": {
            "class_type": "SaveImage",
            "inputs": {
                "images": ["9", 0],
                "filename_prefix": "SUGT/su-master-smoke",
            },
        },
    }


def find_history(prompt_id):
    """读取生成历史，兼容单条与全量两种端点。"""
    try:
        data = json.loads(http_get("/history/" + prompt_id))
        if prompt_id in data:
            return data[prompt_id]
    except Exception:
        pass
    try:
        data = json.loads(http_get("/history?max_items=20"))
        if prompt_id in data:
            return data[prompt_id]
    except Exception:
        pass
    return None


def main():
    os.makedirs(OUT_DIR, exist_ok=True)
    print("目标:", BASE)
    print("分辨率: %dx%d | seed=%d | steps=8 cfg=1.0 euler/simple" % (WIDTH, HEIGHT, SEED))

    try:
        stats = json.loads(http_get("/system_stats"))
        dev = stats.get("devices", [{}])[0]
        print("GPU: %s | VRAM free %.2f GB" % (
            dev.get("name", "?"), dev.get("vram_free", 0) / 1024 ** 3))
    except Exception as exc:
        print("显存前置读取失败(不阻断):", exc)

    try:
        result = http_post_json(
            "/prompt",
            {"prompt": build_prompt(), "client_id": "sugt-sdet-smoke-01"},
        )
    except urllib.error.HTTPError as exc:
        body = exc.read().decode("utf-8", "replace")
        print("提交失败 HTTP %s: %s" % (exc.code, body[:2000]))
        return 1
    except Exception as exc:
        print("提交失败:", exc)
        return 1

    if "prompt_id" not in result:
        print("提交返回异常:", json.dumps(result, ensure_ascii=False)[:2000])
        return 1

    prompt_id = result["prompt_id"]
    print("已提交 prompt_id =", prompt_id)
    if result.get("node_errors"):
        print("节点警告:", json.dumps(result["node_errors"], ensure_ascii=False)[:1500])

    start = time.time()
    last_report = 0
    while time.time() - start < TIMEOUT_SEC:
        hist = find_history(prompt_id)
        if hist:
            status = hist.get("status", {}) or {}
            if status.get("completed") or hist.get("outputs"):
                elapsed = time.time() - start
                print("生成完成，耗时 %.1fs" % elapsed)
                images = []
                for _node, out in (hist.get("outputs") or {}).items():
                    for item in out.get("images", []) or []:
                        images.append(item)
                if not images:
                    print("历史已完成但未找到图片输出")
                    return 1
                for item in images:
                    query = urllib.parse.urlencode({
                        "filename": item.get("filename", ""),
                        "subfolder": item.get("subfolder", ""),
                        "type": item.get("type", "output"),
                    })
                    raw = http_get("/view?" + query, timeout=60)
                    target = os.path.join(OUT_DIR, item.get("filename", "output.png"))
                    with open(target, "wb") as handle:
                        handle.write(raw)
                    print("已取回: %s (%.1f KB)" % (target, len(raw) / 1024))
                return 0
            if status.get("status_str") == "error":
                print("生成报错:", json.dumps(status, ensure_ascii=False)[:2000])
                return 1
        elapsed = time.time() - start
        if elapsed - last_report >= 20:
            print("  等待中 %.0fs ..." % elapsed)
            last_report = elapsed
        time.sleep(3)

    print("超时 %ds 未完成，prompt_id=%s 可稍后查询" % (TIMEOUT_SEC, prompt_id))
    return 1


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("已中断")
        sys.exit(1)
    except Exception as exc:
        print("解析/运行异常:", repr(exc))
        sys.exit(2)
