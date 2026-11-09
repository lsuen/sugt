#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""SUGT ComfyUI 脚本共用 HTTP 适配器：用 curl 通道替换 urllib 直连。

编写人员：孙测测
作用用途：
  本机 Python socket 直连内网 ComfyUI 被系统网络策略拦截（OSError
  Errno 65 / No route to host），而 curl 通道可达。本模块 monkey-patch
  ``urllib.request.urlopen``，让既有 v0.7 / 试片脚本无需改 workflow 即可
  继续运行。仅替换传输层，不改变请求 URL、方法、头部与请求体。
适用项目：SUGT 桌面精灵 ComfyUI 素材链路（本地内网，非公网服务）
创建日期：2026-11-10
复用方式：
  ``import comfyui_curl_http; comfyui_curl_http.install()``
  必须在导入 comfyui_pet_v07.py 之后、发起任何请求之前调用。
退出码约定：本模块不直接执行；异常以 OSError 抛出，由调用方按原约定处理。
"""
import io
import subprocess
import urllib.request

_CURL = "curl"


class CurlResponse(io.BytesIO):
    """模拟 ``urlopen`` 返回值的最小响应对象，支持 with 与 read/status。"""

    def __init__(self, payload, status=200):
        super().__init__(payload)
        self.status = status

    def __enter__(self):
        return self

    def __exit__(self, *exc_info):
        self.close()
        return False


def _split_target(target, data):
    """把 urlopen 的字符串或 Request 入参拆成 curl 所需四元组。"""
    if isinstance(target, urllib.request.Request):
        return (
            target.full_url,
            target.get_method(),
            target.data,
            dict(target.header_items()),
        )
    if isinstance(target, str):
        return target, "GET", data, {}
    raise TypeError("不支持的 urlopen 入参类型: %s" % type(target).__name__)


def curl_urlopen(target, data=None, timeout=30):
    """以 curl 发起请求，返回与 urlopen 兼容的响应对象。"""
    url, method, body, headers = _split_target(target, data)
    args = [
        _CURL, "-s", "--noproxy", "*",
        "--max-time", str(max(1, int(timeout))),
        "-X", method,
    ]
    for key, value in headers.items():
        args.extend(["-H", "%s: %s" % (key, value)])
    if body is not None:
        args.extend(["--data-binary", "@-"])
    args.append(url)

    raw_body = body if isinstance(body, bytes) else None
    try:
        proc = subprocess.run(
            args,
            input=raw_body,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=max(1, int(timeout)) + 10,
            check=False,
        )
    except subprocess.TimeoutExpired as exc:
        raise OSError("curl 请求超时: %s %s" % (method, url)) from exc

    if proc.returncode != 0:
        reason = proc.stderr.decode("utf-8", "replace").strip()
        raise OSError("curl 请求失败(rc=%d): %s %s %s" % (
            proc.returncode, method, url, reason))
    return CurlResponse(proc.stdout)


def install():
    """把 urllib.request.urlopen 替换为 curl 实现，幂等。"""
    current = urllib.request.urlopen
    if getattr(current, "__sugt_curl__", False):
        return current
    setattr(curl_urlopen, "__sugt_curl__", True)
    urllib.request.urlopen = curl_urlopen
    return curl_urlopen
