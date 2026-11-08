#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT 小su 桌面精灵素材生成：本地 ComfyUI 文生图定稿 + 图生图状态帧。

编写人员：孙测测
作用用途：一次提交批量产出角色定稿候选与各状态差分帧，验证本地素材闭环
适用项目：SUGT 桌面精灵素材链路
创建日期：2026-11-08
复用方式：python3 comfyui_pet_assets.py master [seed ...]
          python3 comfyui_pet_assets.py edit <参考图绝对路径> [state ...]
          环境变量 SUGT_COMFYUI_BASE 覆盖服务地址，SUGT_PET_OUT 覆盖输出目录
退出码约定：0 全部生成并取回 / 1 提交或生成失败 / 2 读取解析异常
"""
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

# 服务地址与输出目录均可通过环境变量覆盖，不绑定某一台机器的绝对路径
BASE = os.environ.get("SUGT_COMFYUI_BASE", "http://192.168.31.84:8188")
_DEFAULT_OUT = os.path.abspath(
    os.path.join(os.path.dirname(__file__), "..", "..", "design", "assets", "generated")
)
OUT_DIR = os.environ.get("SUGT_PET_OUT", _DEFAULT_OUT)
TIMEOUT_SEC = 900

# 权重均为 fp8 / 4steps / 8steps 量化版，针对 16GB 显存配置，禁止改全精度
UNET_T2I = "qwen_image_fp8_e4m3fn.safetensors"
UNET_EDIT = "qwen_image_edit_2509_fp8_e4m3fn.safetensors"
LORA_T2I = "Qwen-Image-Lightning-8steps-V1.0.safetensors"
LORA_EDIT = "Qwen-Image-Edit-2509-Lightning-4steps-V1.0-bf16.safetensors"
CLIP_NAME = "qwen_2.5_vl_7b_fp8_scaled.safetensors"
VAE_NAME = "qwen_image_vae.safetensors"

# 定稿正向：去掉会被误解为天使光环的措辞，强化全身含脚、纯白底、核心留空
POSITIVE = (
    "日系动画赛璐璐官方设定稿，高清干净线稿，硬边二段阴影，纯平涂色块，"
    "均匀深蓝黑线稿，无渐变无喷枪无3D无噪点。"
    "发顶一道弧形高光反光带（只是头发反光，绝不是光环、不是天使环）。"
    "眼睛四层：深虹膜环、雾蓝中调、亮色下弦、双白高光，粗上睫毛。"
    "角色小su：中性纤细体型，正面全身站姿，从头顶到脚底完整可见，"
    "双脚完整不被裁切，人物居中，四周留10%纯白安全边距。"
    "雾蓝色短波波头，发梢卷成信号波纹；头顶一撮S形呆毛，尖端一枚小薄荷色光点；"
    "雾蓝色眼睛，浅暖肤色，浅笑，头微侧。"
    "米白色立领短外套，深蓝黑拼接，细青色滚边；"
    "胸口正中一枚细金属框正六边形空白徽章，徽章内部纯色，无图案无符号无发光纹样。"
    "身后两条半透明浅蓝色数据流飘带，末端小方块像素点，少量六边形粒子。"
    "纯白单色平涂背景，clean flat solid pure white background，"
    "no pattern, no checkerboard, no grid, no scenery, no floor shadow。"
    "竖版全身构图，full body head to toe, entire figure visible including both feet。"
)
NEGATIVE = (
    "3D, 写实, 照片, 厚涂, 渐变, 柔光, 发光滤镜, 景深, 投影, 噪点, 水彩, "
    "复杂背景, 图案背景, 棋盘格, checkerboard, grid, 几何纹样, 地面阴影, "
    "天使光环, halo, 光环, 翅膀, wings, 羽毛, "
    "文字, 水印, 签名, 多人, 肥胖, 矮壮, 过度幼态Q版, 兽耳, 猫耳, "
    "武器, 多余肢体, 手指畸形, 六指, 裁切, 半身, 膝上构图, 脚被裁掉, 低分辨率, 脏边"
)

# 状态差分：同一张定稿做指令编辑，锁死外观，只改动作与表情
EDIT_BASE = (
    "严格保持这个角色的外观、发型、发色、服装、配色、构图、背景、画风完全一致，"
    "只做出以下改变：{change}。"
    "仍然是正面全身站姿、纯白背景、赛璐璐硬边平涂、双脚完整可见，"
    "胸口六边形徽章保持空白无图案。"
)
EDIT_JOBS = {
    # idle 与其余状态帧同源同背景（纯白），避免直接用带有棋盘底纹的文生图定稿
    "idle": ("恢复最自然放松的站姿，双手自然垂在身体两侧，双眼睁开，浅笑，"
             "背景保持纯白单色，不要添加任何新元素", 20261114),
    "typing": ("双手抬到身前做出轻敲键盘的动作，视线略向下，表情专注，"
               "身侧浮现几枚半透明小方块数据粒子", 20261110),
    "done": ("轻轻抬起一只手，笑容更明显，眼睛明亮，"
             "周围浮现少量六边形光点", 20261111),
    "error": ("头微微歪向一侧，眉头轻蹙，表情略困惑，双手自然下垂", 20261112),
    "sleep": ("双眼轻轻闭合，头微低，表情放松安静，"
              "周围浮着两三枚半透明小方块", 20261113),
}


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


def upload_image(path):
    """把本地定稿上传到 ComfyUI input 目录，返回 LoadImage 可用的文件名。"""
    boundary = "----sugt" + uuid.uuid4().hex
    with open(path, "rb") as handle:
        raw = handle.read()
    body = (
        ("--%s\r\nContent-Disposition: form-data; name=\"image\"; filename=\"%s\"\r\n"
         "Content-Type: image/png\r\n\r\n" % (boundary, os.path.basename(path))).encode()
        + raw + b"\r\n"
        + ("--%s\r\nContent-Disposition: form-data; name=\"overwrite\"\r\n\r\ntrue\r\n"
           % boundary).encode()
        + ("--%s--\r\n" % boundary).encode()
    )
    req = urllib.request.Request(
        BASE + "/upload/image", data=body,
        headers={"Content-Type": "multipart/form-data; boundary=" + boundary},
    )
    with urllib.request.urlopen(req, timeout=120) as resp:
        info = json.loads(resp.read().decode("utf-8"))
    name = info.get("name") or os.path.basename(path)
    sub = info.get("subfolder") or ""
    return os.path.join(sub, name).replace("\\", "/") if sub else name


def common_t2i(prompt):
    """文生图共享骨架：加载器 + 正负向条件 + 空白 latent。"""
    prompt["1"] = {
        "class_type": "UNETLoader",
        "inputs": {"unet_name": UNET_T2I, "weight_dtype": "default"},
    }
    prompt["2"] = {
        "class_type": "LoraLoaderModelOnly",
        "inputs": {"model": ["1", 0], "lora_name": LORA_T2I, "strength_model": 1.0},
    }
    prompt["3"] = {
        "class_type": "CLIPLoader",
        "inputs": {"clip_name": CLIP_NAME, "type": "qwen_image", "device": "default"},
    }
    prompt["4"] = {"class_type": "VAELoader", "inputs": {"vae_name": VAE_NAME}}
    prompt["5"] = {
        "class_type": "CLIPTextEncode",
        "inputs": {"text": POSITIVE, "clip": ["3", 0]},
    }
    prompt["6"] = {
        "class_type": "CLIPTextEncode",
        "inputs": {"text": NEGATIVE, "clip": ["3", 0]},
    }
    prompt["7"] = {
        "class_type": "EmptySD3LatentImage",
        "inputs": {"width": 1024, "height": 1536, "batch_size": 1},
    }
    return prompt


def build_master(seeds):
    """一次提交产出多张定稿候选，共享模型加载，避免反复换入显存。"""
    prompt = common_t2i({})
    for index, seed in enumerate(seeds):
        base = 10 + index * 3
        prompt[str(base)] = {
            "class_type": "KSampler",
            "inputs": {
                "model": ["2", 0], "seed": seed, "steps": 8, "cfg": 1.0,
                "sampler_name": "euler", "scheduler": "simple",
                "positive": ["5", 0], "negative": ["6", 0],
                "latent_image": ["7", 0], "denoise": 1.0,
            },
        }
        prompt[str(base + 1)] = {
            "class_type": "VAEDecode",
            "inputs": {"samples": [str(base), 0], "vae": ["4", 0]},
        }
        prompt[str(base + 2)] = {
            "class_type": "SaveImage",
            "inputs": {
                "images": [str(base + 1), 0],
                "filename_prefix": "SUGT/pet-master-s%d" % seed,
            },
        }
    return prompt


def build_edit(ref_name, states):
    """以定稿为参考做指令编辑，一次提交产出全部状态帧。"""
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
        "7": {
            "class_type": "VAEEncode",
            "inputs": {"pixels": ["6", 0], "vae": ["5", 0]},
        },
        "8": {
            "class_type": "CLIPTextEncode",
            "inputs": {"text": "", "clip": ["4", 0]},
        },
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
                "filename_prefix": "SUGT/pet-%s" % state,
            },
        }
    return prompt


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


def submit_and_fetch(prompt, label):
    """提交任务、轮询历史、取回全部图片产物。"""
    os.makedirs(OUT_DIR, exist_ok=True)
    print("模式:", label, "| 目标:", BASE, "| 输出:", OUT_DIR, flush=True)
    try:
        result = http_post_json(
            "/prompt", {"prompt": prompt, "client_id": "sugt-pet-assets-01"}
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
        print("节点警告:", json.dumps(result["node_errors"], ensure_ascii=False)[:2000], flush=True)

    start = time.time()
    last_report = 0
    while time.time() - start < TIMEOUT_SEC:
        hist = find_history(prompt_id)
        if hist:
            status = hist.get("status", {}) or {}
            if status.get("status_str") == "error":
                print("生成报错:", json.dumps(status, ensure_ascii=False)[:3000], flush=True)
                return 1
            if status.get("completed") or hist.get("outputs"):
                print("生成完成，耗时 %.1fs" % (time.time() - start), flush=True)
                images = []
                for _node, out in (hist.get("outputs") or {}).items():
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
                    raw = http_get("/view?" + query, timeout=90)
                    target = os.path.join(OUT_DIR, item.get("filename", "output.png"))
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


def main():
    if len(sys.argv) < 2 or sys.argv[1] not in ("master", "edit"):
        print(__doc__)
        return 2
    mode = sys.argv[1]
    if mode == "master":
        seeds = [int(x) for x in sys.argv[2:]] or [20261110, 20261111, 20261112]
        return submit_and_fetch(build_master(seeds), "master seeds=%s" % seeds)
    if len(sys.argv) < 3:
        print("edit 模式需要参考图路径")
        return 2
    ref_path = sys.argv[2]
    if not os.path.isfile(ref_path):
        print("参考图不存在:", ref_path)
        return 2
    states = sys.argv[3:] or list(EDIT_JOBS)
    unknown = [x for x in states if x not in EDIT_JOBS]
    if unknown:
        print("未知状态:", unknown, "| 可选:", list(EDIT_JOBS))
        return 2
    ref_name = upload_image(ref_path)
    print("已上传参考图:", ref_name, flush=True)
    return submit_and_fetch(build_edit(ref_name, states), "edit states=%s" % states)


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print("已中断")
        sys.exit(1)
    except Exception as exc:
        print("解析/运行异常:", repr(exc))
        sys.exit(2)
