# SUGT

A local AI gateway for Windows. One place for upstream models, one-click agent takeover, and (in the full build) a Skills console you can download into, manage locally, and remount without losing your library.

If you use Claude Code / Codex / OpenCode with Chinese or self-hosted providers, you should not have to re-edit Base URLs in every client. Point agents at localhost; swap upstreams in one panel.

| | |
| --- | --- |
| Version | 1.0.0 |
| Author | Sun Wenlong · 异常设计 |
| Open source | Feature core under [Apache-2.0](LICENSE) |
| Current binary | [Full trial build](https://github.com/lsuen/sugt/releases/tag/v1.0.0-store-trial) (includes the skill console) |

中文说明：[README.md](README.md)

## Why

- API keys scattered across OpenAI, Anthropic, ModelScope, Volcengine, local models…
- Every agent wants its own Base URL and auth setup
- Skills live in different folders with no shared discover / mount / keep flow

SUGT is intentionally small: a compatible local gateway, takeover for common agents, and a skill control plane in the full release.

## A note from the author (please read)

I will keep offering a **free** way to use this. The trial build is not a trap to force payment later.

Right now the Release asset is a **time-limited full build** because I still have ideas I want to finish, and I want more code review before I call anything “done.” It already helps with real setup pain, so I published it anyway. That felt like the responsible thing: if it can help, don’t sit on it.

When things are steadier, I will publish a **permanent full build** (no trial countdown). The open feature-core source can also be built without trial injection; the author’s packaging/trial scripts are simply not in the public tree.

Some modules (mainly the skill console source) are still private for the same reason — polish and review — **not** because I plan to keep them closed forever. I intend to open the rest over time, and eventually all of it.

### Why this release took courage

I’m not a young founder with investors, a team, or a big account behind me. I’m older than most people who casually drop repos on GitHub. The last few years have been hard — money, energy, and the quiet fear that work done alone might never be seen.

SUGT began as something I needed for my own days. Putting it online meant saying out loud: this might matter to someone else. That was not easy. I finally gathered the courage to open-source what I could, and to ship a usable build even while unfinished pieces remain.

If it helps you, a star or a short Issue is enough. If it doesn’t, that’s okay. I’m not asking for pity — only for a fair look, and maybe a little patience while I keep improving it.

## Download

Windows x64 full trial (~90 days, machine-bound):

**[SUGT 1.0.0 store trial](https://github.com/lsuen/sugt/releases/tag/v1.0.0-store-trial)**

Config defaults to `Documents\.sugt\` (`SUGT_CONFIG_DIR` overrides it). Feedback: [Issues](https://github.com/lsuen/sugt/issues).

## Screenshots

Console — start/stop, traffic, agent endpoints.

![Console](docs/images/client.png)

Models — providers, default model, connectivity test.

![Models](docs/images/client-llm.png)

Clients — discover agents, take over or release.

![Clients](docs/images/client-agents.png)

Skills (full build) — local mount and repo discover.

![Local skills](docs/images/client-skills-local.png)

![Discover skills](docs/images/client-skills-store.png)

## Features

- Local gateway: Chat Completions, Anthropic Messages, OpenAI Responses (with fallback)
- Protocol bridging between Anthropic and OpenAI shapes
- Provider presets, enable/disable, default switch, test
- One-click takeover for Claude Code / Codex / OpenCode and similar clients
- Skill console in the full binary (refresh, install, mount; library stays even if you unmount)
- Desktop UI (Tauri) and `sugt-cli`

More: [open-source policy](docs/open-source.md) · [provider notes](docs/provider-vendors.md) · [build from source](docs/build.md)

## Build (feature core)

Windows 10/11, Node 18+, Rust 1.78+, WebView2.

```cmd
npm install
npm run check
npm run dev:app
```

See [docs/build.md](docs/build.md). Public tree = feature core. For a no-trial full binary, build from the author’s full tree with a self/dev edition (trial injection is packaging-time, not a permanent lock on the idea of the product).

```cmd
sugt-cli status
sugt-cli serve --host 127.0.0.1 --port 8787
```

## License

Feature core: Apache-2.0. Remaining private modules are temporary and intended to be opened over time. See [LICENSE](LICENSE), [NOTICE](NOTICE), and [docs/open-source.md](docs/open-source.md).
