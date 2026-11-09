#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT 桌面精灵：新动作关键姿态能力核实试片（不进包）。

编写人员：孙测测
作用用途：
  复用 v0.7 的 Qwen-Image-Edit 洋红底链路，对「转头 / 歪头 / 伸懒腰」等
  1000 帧扩容所需的新动作各出一张关键姿态，用于核实模型能否在保持角色
  一致性的前提下产出真实动作语义。产物只落临时目录，绝不写 src/assets/pet。
适用项目：SUGT 桌面精灵 1000 帧扩容前置能力核实
创建日期：2026-11-09
复用方式：
  python3 comfyui_pet_keyposes_trial.py --ref <洋红底定稿图> \
      --actions head-turn head-tilt stretch --out /tmp/sugt-pet-keyposes-trial
  环境变量 SUGT_COMFYUI_BASE 覆盖服务地址。
  本机 Python socket 直连内网被系统拦截，HTTP 统一走 curl 适配器（见下）。
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
import comfyui_curl_http  # noqa: E402  标准库之后、业务模块之前显式导入

# 新动作关键姿态：放宽「正面站姿」，但锁死身体构图、手臂数量与画风。
TRIAL_BASE = (
    "严格保持这个角色的外观、发型、发色、服装、配色、人物大小、画面位置、画风完全一致，"
    "只做出以下改变：{change}。"
    "身体保持正面站姿、完整双腿与双脚、双臂自然垂在身侧且只有两条手臂两只手，"
    "绝不新增、复制或残留多余的手臂与手，不要把手臂在新位置再画一遍。"
    "背景必须保持与参考图完全相同的纯净洋红纯色，每一个背景像素都是同一个颜色，"
    "绝对不能有渐变、图案、纹理、网格、地面阴影或杂色，也不要把洋红色染到人物上。"
    "保持赛璐璐硬边平涂、干净均匀线稿、眼睛颜色 #7FB4E8，"
    "胸口六边形徽章保持空白无图案，不要添加任何文字、水印或签名。"
)

TRIAL_JOBS = {
    "head-turn": (
        "头部明显转向画面左侧约三十度，视线跟着看向左侧，身体的肩线、双臂与双脚"
        "完全保持正面不动，不要画成侧面全身像",
        20261201,
    ),
    "head-tilt": (
        "头部轻轻歪向观众一侧约十五度，一侧肩膀微微抬高，眼睛弯起带着好奇的表情，"
        "身体其余部分与构图完全不动",
        20261202,
    ),
    "stretch": (
        "整个人像刚睡醒一样轻轻伸展：双肩微微上提并向后拉开，脊椎舒展，"
        "双臂仍然自然垂在身侧不要抬起举过头顶，头微微后仰，嘴巴小幅度打哈欠",
        20261203,
    ),
    "yawn": (
        "头微微后仰，嘴巴小幅度张开打哈欠，眼睛半闭，肩膀放松下沉，"
        "身体其余部分与构图完全不动",
        20261204,
    ),
    "look-around": (
        "头部转向画面右侧约二十度，视线看向侧上方像在张望，"
        "身体的肩线、双臂与双脚完全保持正面不动",
        20261205,
    ),
}


def load_base_module():
    """按文件路径加载 v0.7 链路模块，复用其 workflow 与取图逻辑。"""
    # 必须在 base 模块发起任何请求前替换传输层，否则 urllib 直连会 Errno 65。
    comfyui_curl_http.install()
    spec = importlib.util.spec_from_file_location("comfyui_pet_v07", _BASE_SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError("无法加载基础模块: %s" % _BASE_SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def write_trace(out_dir, actions, ok, detail):
    """把本次试片结果写成 Markdown 留痕，便于后续许可证/能力核实引用。"""
    _TRACES.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    path = _TRACES / ("精灵关键姿态试片-%s.md" % stamp)
    lines = [
        "# 桌面精灵关键姿态能力核实试片（不进包）",
        "",
        "- 执行时间：%s" % time.strftime("%Y-%m-%d %H:%M:%S"),
        "- 编写人员：孙测测",
        "- 服务地址：%s" % os.environ.get("SUGT_COMFYUI_BASE", "http://192.168.31.84:8188"),
        "- 动作：%s" % ", ".join(actions),
        "- 输出目录：%s（临时目录，产物不进包）" % out_dir,
        "- 结论：%s" % ("通过" if ok else "不通过"),
        "",
        "## 明细",
        "",
        "```",
        detail,
        "```",
        "",
        "> 合规边界：本试片仅核实模型能力，未闭环 Qwen-Image / Qwen-Image-Edit / "
        "Lightning LoRA 随包分发许可前，产物不得写入 src/assets/pet。",
        "",
    ]
    path.write_text("\n".join(lines), encoding="utf-8")
    return path


def main():
    parser = argparse.ArgumentParser(description="桌面精灵新动作关键姿态试片（不进包）")
    parser.add_argument("--ref", required=True, help="洋红底定稿图绝对路径")
    parser.add_argument("--actions", nargs="+", default=list(TRIAL_JOBS)[:3],
                        help="要生成的动作名，默认前三个")
    parser.add_argument("--out", default="/tmp/sugt-pet-keyposes-trial",
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
    unknown = [name for name in args.actions if name not in TRIAL_JOBS]
    if unknown:
        print("未知动作:", unknown, "可选:", list(TRIAL_JOBS), flush=True)
        return 2
    out_dir.mkdir(parents=True, exist_ok=True)

    try:
        base = load_base_module()
    except Exception as exc:
        print("加载 v0.7 链路失败:", exc, flush=True)
        return 2

    # 覆盖基础模块的动作表、提示词模板与输出目录，其余 workflow 完全复用。
    base.EDIT_JOBS = {name: TRIAL_JOBS[name] for name in args.actions}
    base.EDIT_BASE = TRIAL_BASE
    base.OUT_DIR = str(out_dir)
    base.TIMEOUT_SEC = 900

    print("试片动作:", ", ".join(args.actions), flush=True)
    print("参考图:", ref, flush=True)
    print("输出目录:", out_dir, "(不进包)", flush=True)
    try:
        ref_name = base.upload_image(str(ref), tag="sugt-keyposes-ref.png")
        prompt = base.build_states(ref_name, args.actions)
        code = base.submit_and_fetch(prompt, "keyposes-trial")
    except Exception as exc:
        code, detail = 1, "生成异常: %s" % exc
    else:
        files = sorted(p.name for p in out_dir.glob("*.png"))
        detail = "产物 %d 张: %s" % (len(files), ", ".join(files))
        print(detail, flush=True)
        if code == 0 and not files:
            code, detail = 1, detail + " / 未取回任何图片"

    trace = write_trace(out_dir, args.actions, code == 0, detail)
    print("留痕:", trace, flush=True)
    return code


if __name__ == "__main__":
    sys.exit(main())
