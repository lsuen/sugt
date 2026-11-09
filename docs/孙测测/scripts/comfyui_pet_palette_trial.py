#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT 桌面精灵：发色 / 下装配色候选试片（不进包）。

编写人员：孙测测
作用用途：
  基于唯一正面定稿图，用 Qwen-Image-Edit 图生图派生三套形象候选，
  用于回答「是否换银灰发」「下装是否换成街舞工装裤」「撞鲸鱼娘」三个
  设计问题。产物只落临时目录，绝不写 src/assets/pet。
适用项目：SUGT 桌面精灵 1000 帧扩容前的形象定稿决策
创建日期：2026-11-10
复用方式：
  python3 comfyui_pet_palette_trial.py --ref <洋红底定稿图> \
      --schemes silver-cargo silver-minimal warm-street \
      --out /tmp/sugt-pet-palette-trial
  环境变量 SUGT_COMFYUI_BASE 覆盖服务地址；HTTP 统一走 curl 适配器。
退出码约定：0 全部成功 / 1 提交或生成失败 / 2 参数或解析异常
"""
import argparse
import importlib.util
import os
import sys
import time
from pathlib import Path

_ROOT = Path(__file__).resolve().parents[3]
_SCRIPTS = _ROOT / "docs" / "孙测测" / "scripts"
_BASE_SCRIPT = _SCRIPTS / "comfyui_pet_v07.py"
_TRACES = _ROOT / "docs" / "孙测测" / "traces"
sys.path.insert(0, str(_SCRIPTS))
import comfyui_curl_http  # noqa: E402

# 与关键姿态试片不同：这里明确允许改发色与下装，其余逐项锁死。
# 锁死项来自 su-mascot.html 色指定表与 v0.7 链路已验证的构图约束。
PALETTE_BASE = (
    "以参考图为唯一角色基准，严格保持脸型、五官位置、头身比、人物大小、"
    "画面构图与全身站姿完全一致，只允许按下面的描述修改{change}。"
    "眼睛颜色必须是 #7FB4E8 雾蓝，虹膜环与双白高光保持不变，禁止改成青绿眼。"
    "头顶那撮翘起的 S 形呆毛形状与位置保持不变，尖端保留一个薄荷绿光点。"
    "保持赛璐璐硬边平涂二段影、均匀深蓝黑线稿、月牙 Angel-ring 高光，"
    "禁止渐变、喷枪、3D 质感、噪点与写实皮肤。"
    "身体保持正面站姿、完整双腿与双脚、双臂自然垂在身侧且只有两条手臂两只手，"
    "绝不新增、复制或残留多余手臂，也不要把手臂在旧位置再画一遍。"
    "背景必须保持与参考图完全相同的纯净洋红纯色，每一个背景像素都是同一个颜色，"
    "绝对不能有渐变、图案、纹理、网格、地面阴影或杂色，也不要把洋红色染到人物上。"
    "胸口六边形徽章保持空白无图案，不要添加任何文字、水印或签名。"
    "服装轮廓必须保持纤细，不要画成宽松臃肿或矮壮的体型。"
)

PALETTE_JOBS = {
    # 主推：银灰发 + 街舞工装裤 + 品牌薄荷绿点缀，一次性和鲸鱼娘拉开。
    "silver-cargo": (
        "把短发整体改成银灰色，发梢略带冷调浅灰、保留原有二段影与月牙高光，"
        "不要染成蓝色也不要变成白色；"
        "把深蓝运动短裤换成炭灰色微宽松直筒工装长裤，两侧各有一个明显的立体贴袋，"
        "裤脚收口，保留纤细腿部剪影，不要画成超宽垮裤；"
        "米白立领拉链外套改成短款并略微敞开，露出里面一件薄荷绿内搭，"
        "袖子的深蓝黑与青色滚边保持不变；白色运动鞋与鞋上小标保持不变",
        20261101,
    ),
    # 对照：只换发色，用于验证「单换发色是否足以解决撞脸」。
    "silver-minimal": (
        "只把短发整体改成银灰色，发梢略带冷调浅灰、保留原有二段影与月牙高光；"
        "服装、配色、鞋、配饰、呆毛、表情全部与参考图逐像素一致，不做任何其他改变",
        20261102,
    ),
    # 极端对冲：暖色系发 + 深色工装裤，与鲸鱼娘的冷蓝完全反向。
    "warm-street": (
        "把短发整体改成浅茶棕色带一点砂金，保留原有二段影与月牙高光，不要偏红；"
        "把深蓝运动短裤换成炭黑色微宽松直筒工装长裤，两侧各有一个明显的立体贴袋，"
        "裤脚收口，保留纤细腿部剪影；"
        "米白立领拉链外套的青色滚边改成薄荷绿，内搭换成薄荷绿，"
        "深蓝黑袖子与白色运动鞋保持不变",
        20261103,
    ),
}


def load_base_module():
    """加载 v0.7 链路，复用 workflow；先把传输层换成 curl 通道。"""
    comfyui_curl_http.install()
    spec = importlib.util.spec_from_file_location("comfyui_pet_v07", _BASE_SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("无法加载基础模块: %s" % _BASE_SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write_trace(out_dir, schemes, ok, detail):
    """把本次形象候选试片结果写成 Markdown 留痕。"""
    _TRACES.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    path = _TRACES / ("精灵形象候选试片-%s.md" % stamp)
    lines = [
        "# 桌面精灵发色 / 下装形象候选试片（不进包）",
        "",
        "- 执行时间：%s" % time.strftime("%Y-%m-%d %H:%M:%S"),
        "- 编写人员：孙测测",
        "- 服务地址：%s" % os.environ.get("SUGT_COMFYUI_BASE", "http://192.168.31.84:8188"),
        "- 候选方案：%s" % ", ".join(schemes),
        "- 输出目录：%s（临时目录，产物不进包）" % out_dir,
        "- 结论：%s" % ("通过" if ok else "不通过"),
        "",
        "## 明细",
        "",
        "```",
        detail,
        "```",
        "",
        "> 合规边界：本试片仅核实形象候选，未闭环 Qwen-Image / Qwen-Image-Edit / "
        "Lightning LoRA 随包分发许可前，产物不得写入 src/assets/pet。",
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def main():
    parser = argparse.ArgumentParser(description="桌面精灵形象候选试片（不进包）")
    parser.add_argument("--ref", required=True, help="洋红底定稿图绝对路径")
    parser.add_argument("--schemes", nargs="+", default=list(PALETTE_JOBS),
                        help="候选方案名，默认三套全出")
    parser.add_argument("--out", default="/tmp/sugt-pet-palette-trial",
                        help="输出目录，默认临时目录；禁止指向 src/assets/pet")
    args = parser.parse_args()

    ref = Path(args.ref).resolve()
    out_dir = Path(args.out).resolve()
    if not ref.is_file():
        print("参考图不存在:", ref, flush=True)
        return 2
    if "src/assets/pet" in str(out_dir).replace("\\", "/"):
        print("拒绝写入进包素材目录:", out_dir, flush=True)
        return 2
    unknown = [name for name in args.schemes if name not in PALETTE_JOBS]
    if unknown:
        print("未知方案:", unknown, "可选:", list(PALETTE_JOBS), flush=True)
        return 2
    out_dir.mkdir(parents=True, exist_ok=True)

    try:
        base = load_base_module()
    except Exception as exc:
        print("加载 v0.7 链路失败:", exc, flush=True)
        return 2

    base.EDIT_JOBS = {name: PALETTE_JOBS[name] for name in args.schemes}
    base.EDIT_BASE = PALETTE_BASE
    base.OUT_DIR = str(out_dir)
    base.TIMEOUT_SEC = 900

    print("候选方案:", ", ".join(args.schemes), flush=True)
    print("参考图:", ref, flush=True)
    print("输出目录:", out_dir, "(不进包)", flush=True)
    try:
        ref_name = base.upload_image(str(ref), tag="sugt-palette-ref.png")
        prompt = base.build_states(ref_name, args.schemes)
        code = base.submit_and_fetch(prompt, "palette-trial")
    except Exception as exc:
        code, detail = 1, "生成异常: %s" % exc
    else:
        files = sorted(p.name for p in out_dir.glob("*.png"))
        detail = "产物 %d 张: %s" % (len(files), ", ".join(files))
        print(detail, flush=True)
        if code == 0 and not files:
            code, detail = 1, detail + " / 未取回任何图片"

    trace = write_trace(out_dir, args.schemes, code == 0, detail)
    print("留痕:", trace, flush=True)
    return code


if __name__ == "__main__":
    sys.exit(main())
