<div align="center">
  <h1>🎙️ vox</h1>
  <p><strong>Voice I/O companion for CLI AI agents</strong></p>
  <p>
    <a href="#features">Features</a> •
    <a href="#installation">Installation</a> •
    <a href="#usage">Usage</a> •
    <a href="#configuration">Configuration</a> •
    <a href="#keybindings">Keybindings</a>
  </p>
  <p>
    <img src="https://img.shields.io/badge/status-alpha-orange" alt="status">
    <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey" alt="platform">
    <img src="https://img.shields.io/badge/license-MIT-blue" alt="license">
  </p>
</div>

---

## Overview

**vox** is a lightweight, cross-platform system tray application that gives any CLI AI agent — or any desktop application — **speech input (ASR)** and **text-to-speech (TTS)** capabilities. It works at the system level, injecting recognized text at the cursor position or reading selected text aloud, without requiring any plugin or integration from the target application.

---

## Features

- 🎤 **Voice Input** — Press a hotkey, speak, and have the transcribed text appear instantly at any cursor position (terminals, editors, browsers, anywhere)
- 🔊 **Text-to-Speech** — Select text and press a hotkey to hear it read aloud; supports selection copy and clipboard modes
- 🧠 **Multiple ASR Engines** — Local inference (whisper.cpp HTTP server, whisper-rs FFI) and cloud engines (OpenAI-compatible, Mimo, Aliyun) with automatic fallback
- 🗣️ **Multiple TTS Engines** — **Free Microsoft Edge TTS** (no API key) and cloud Mimo TTS, with voice/rate/volume/pitch configuration
- 🪶 **Minimal Footprint** — Pure system tray icon, no main window, zero CPU when idle
- 🌍 **Global Hotkeys** — Fully customizable keybindings for all actions
- 💻 **Cross-Platform** — Windows, macOS and Linux

---

## Installation

### Prerequisites

- [Rust toolchain](https://rustup.rs/) 1.75+
- A microphone and speakers/headphones

### Build from source

```bash
git clone https://github.com/your-username/vox.git
cd vox
cargo build --release
./target/release/vox
```

### Quick start

```bash
# (Optional) Start a local ASR server so voice input works offline / free:
#   whisper.cpp:  ./whisper-server -m ggml-tiny.bin --port 8080
#   or any OpenAI-compatible server (faster-whisper, LocalAI, ...) pointed at
#   [asr.openai].base_url in the config.
# If no local server runs, vox falls back through the configured engines.

# Run the app
cargo run --release

# The tray icon appears in your system tray.
# Press Alt+` to start recording, say something, press Alt+` again.
# The transcribed text appears at your cursor position.
# Press Alt+T with text selected to hear it read aloud (Edge TTS, no key).
```

---

## Usage

### Voice Input (ASR)

1. Place your cursor in any text field (terminal, editor, browser)
2. Press `Alt+`` to start recording (tray icon turns red)
3. Speak into your microphone
4. Press `Alt+`` again to stop recording
5. The recognized text is automatically injected at your cursor

### Text-to-Speech (TTS)

1. Select text in any application
2. Press `Alt+T` to hear it read aloud
3. Or right-click the tray icon → `TTS Input` → switch to `Clipboard` mode to read clipboard content

---

## Keybindings

| Action             | Default                                      | Description                                          |
| ------------------ | -------------------------------------------- | ---------------------------------------------------- |
| Record             | <kbd>Alt</kbd>+<kbd>`</kbd>                  | Hold to record (push-to-talk) or press to toggle     |
| Switch ASR engine  | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> | Cycle through available engines                      |
| Switch inject mode | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> | Toggle keyboard / clipboard injection                |
| TTS trigger        | <kbd>Alt</kbd>+<kbd>T</kbd>                  | Read selected text (or clipboard) aloud              |

The record hotkey behavior depends on the **Record Mode** (settable in the
tray menu or Settings):
- **Push-to-Talk** (default): hold `Alt+`` to record, release to stop & transcribe
- **Toggle**: press `Alt+`` to start, press again to stop & transcribe

All keybindings are configurable in `config.toml`.

---

## Tray Menu

```
ASR Engine   ▸  whisper-cpp / openai / mimo / aliyun / whisper-local   (✓ active)
Inject Mode  ▸  Keyboard / Clipboard                                    (✓ active)
Record Mode  ▸  Push-to-Talk (hold) / Toggle (press)                    (✓ active)
─────────────
TTS Engine   ▸  edge-tts / mimo-tts                                      (✓ active)
TTS Input    ▸  Selection (Ctrl+C) / Clipboard                           (✓ active)
─────────────
Toggle Recording
Settings…
Quit
```

The active option in each submenu is checkmarked. The menu and tooltip are
rebuilt live from a plain-data model whenever state changes — no restart
needed when you switch engines or modes.

---

## Configuration

Configuration file location:

| Platform | Path                                                      |
| -------- | --------------------------------------------------------- |
| Windows  | `%APPDATA%\vox\vox\config\config.toml`                  |
| macOS    | `~/Library/Application Support/com.vox/vox/config.toml` |
| Linux    | `~/.config/vox/config.toml`                             |

```toml
[hotkey]
record_toggle = "Alt+`"
engine_switch = "Alt+Shift+E"
inject_mode_switch = "Alt+Shift+V"
tts_trigger = "Alt+T"

[asr]
primary_engine = "whisper-cpp"          # local, no key
fallback_engines = ["openai"]

[asr.whisper_cpp]
base_url = "http://127.0.0.1:8080"      # whisper.cpp HTTP server

[asr.openai]
base_url = "https://api.openai.com/v1"  # or a local OpenAI-compatible server
api_key = ""
model = "whisper-1"

[asr.mimo]
base_url = "https://token-plan-cn.xiaomimimo.com/v1"
api_key = ""
model = "mimo-v2.5-asr"

[inject]
mode = "keyboard"

[tts]
primary_engine = "edge-tts"             # free, no API key
input_mode = "selection"

[tts.edge]
voice = "zh-CN-XiaoxiaoNeural"
rate = "+0%"
volume = "+0%"
pitch = "+0Hz"

[tts.mimo]
model = "mimo-v2.5-tts"
voice = "default"
speed = 1.0
```

---

## Available Engines

### ASR

| Engine                                      | Type                            | Status                                               |
| ------------------------------------------- | ------------------------------- | ---------------------------------------------------- |
| **whisper.cpp** (`whisper-cpp`)     | Local (HTTP server)             | ✅ Default — no key, no FFI                         |
| **OpenAI-compatible** (`openai`)    | Cloud / Local (REST, multipart) | ✅`base_url` configurable for localhost            |
| **Mimo ASR** (`mimo`)               | Cloud (multimodal chat)         | ✅ Needs API key                                     |
| **Aliyun ASR** (`aliyun`)           | Cloud (一句话识别)              | ✅ Needs appkey + token                              |
| **Whisper Local** (`whisper-local`) | Local (whisper-rs FFI)          | ⚠️ Requires`--features whisper-local` + libclang |

### TTS

| Engine                            | Type                 | Status                                  |
| --------------------------------- | -------------------- | --------------------------------------- |
| **Edge TTS** (`edge-tts`) | Cloud (free, no key) | ✅ Default — Microsoft Edge Read Aloud |
| **Mimo TTS** (`mimo-tts`) | Cloud (neural TTS)   | ✅ Needs API key                        |

---

## Project Structure

```
vox/
├── Cargo.toml
├── SPEC.md              # Specification
├── PLAN.md              # Implementation plan
└── src/
    ├── main.rs          # Event loop, CLI subcommands, engine wiring
    ├── app/             # State machine & global hotkeys
    ├── asr/             # AsrEngine trait + manager (fallback) + engines
    │                     whisper_cpp / openai / mimo / aliyun / whisper_local
    ├── audio/           # Microphone capture, WAV encode, resampling
    ├── config/          # TOML configuration (serde-default backwards compat)
    ├── inject/          # Text injection (keyboard/clipboard) + clipboard snapshot
    ├── settings/        # egui settings window (pure view over Config snapshot)
    ├── tray/            # System tray + menu built from a MenuModel
    └── tts/             # TtsEngine trait + manager + engines
                          edge_tts / mimo_tts + rodio playback
```

---

## Development

```bash
# Build
cargo build

# Run with debug logging
RUST_LOG=debug cargo run

# Run tests
cargo test

# Build release
cargo build --release

# Debug CLI subcommands (no GUI):
cargo run -- transcribe <audio.wav>          # test ASR with a file
cargo run -- inject "<text>" --mode keyboard # test text injection
cargo run -- tts "<text>" [out.mp3]          # test TTS (writes file + plays)
```

---

## License

MIT

---

<br>

<div align="center">
  <h1>🎙️ vox</h1>
  <p><strong>CLI AI Agent 的语音 I/O 伴侣</strong></p>
</div>

## 概述

**vox** 是一个轻量级、跨平台的系统托盘应用，为任何 CLI AI Agent——以及任何桌面应用——提供**语音输入（ASR）**和**文字转语音（TTS）**能力。它在系统层面工作，将识别出的文字直接注入光标位置，或朗读选中的文字，无需目标应用安装任何插件。

---

## 功能

- 🎤 **语音输入** — 按快捷键说话，识别文字即刻出现在任意光标位置（终端、编辑器、浏览器……）
- 🔊 **文字转语音** — 选中文字按快捷键自动朗读；支持选中文字和剪贴板两种模式
- 🧠 **多 ASR 引擎** — 本地推理（whisper.cpp HTTP 服务、whisper-rs FFI）与云端引擎（OpenAI 兼容、Mimo、阿里云）自动 fallback
- 🗣️ **多 TTS 引擎** — **免费微软 Edge TTS**（无需 API Key）与云端 Mimo TTS，支持音色/语速/音量/音调配置
- 🪶 **极致轻量** — 纯系统托盘图标，无主窗口，空闲时零 CPU 占用
- 🌍 **全局快捷键** — 所有操作快捷键完全自定义
- 💻 **跨平台** — 支持 Windows、macOS 和 Linux

---

## 安装

### 前置条件

- [Rust 工具链](https://rustup.rs/) 1.75+
- 麦克风和音箱/耳机

### 从源码编译

```bash
git clone https://github.com/your-username/vox.git
cd vox
cargo build --release
./target/release/vox
```

### 快速上手

```bash
# （可选）启动本地 ASR 服务，实现离线/免费语音输入：
#   whisper.cpp:  ./whisper-server -m ggml-tiny.bin --port 8080
#   或任意 OpenAI 兼容服务（faster-whisper、LocalAI……），把
#   [asr.openai].base_url 指向它即可。
# 若无本地服务，vox 会按配置的引擎链自动 fallback。

# 运行
cargo run --release

# 托盘图标出现后，按 Alt+` 开始录音，说完再按 Alt+` 停止
# 识别的文字自动出现在光标位置
# 选中文字按 Alt+T 朗读（Edge TTS，免密钥）
```

---

## 使用说明

### 语音输入（ASR）

1. 将光标放在任意文本输入位置（终端、编辑器、浏览器等）
2. 按 `Alt+`` 开始录音（托盘图标变红）
3. 对着麦克风说话
4. 再按 `Alt+`` 停止录音
5. 识别结果文字自动注入光标位置

### 文字转语音（TTS）

1. 在任意应用中选中文字
2. 按 `Alt+T` 自动朗读
3. 或右键托盘图标 → `TTS Input` → 切换到 `Clipboard` 模式读取剪贴板内容

---

## 快捷键

| 操作          | 默认按键                                     | 说明                                      |
| ------------- | -------------------------------------------- | ----------------------------------------- |
| 录音          | <kbd>Alt</kbd>+<kbd>`</kbd>                  | 按住录音（push-to-talk）或按一下切换      |
| 切换 ASR 引擎 | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> | 循环切换可用引擎                          |
| 切换注入模式  | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> | 键盘模拟 / 剪贴板粘贴                     |
| TTS 触发      | <kbd>Alt</kbd>+<kbd>T</kbd>                  | 朗读选中文字（或剪贴板内容）              |

录音热键的行为取决于 **Record Mode**（可在托盘菜单或设置里切换）：
- **Push-to-Talk**（默认）：按住 `Alt+`` 录音，松手停止并识别
- **Toggle**：按一下 `Alt+`` 开始，再按一下停止并识别

所有快捷键可在 `config.toml` 中自定义。

---

## 托盘菜单

```
ASR Engine   ▸  whisper-cpp / openai / mimo / aliyun / whisper-local   (✓ 当前)
Inject Mode  ▸  Keyboard / Clipboard                                    (✓ 当前)
Record Mode  ▸  Push-to-Talk (hold) / Toggle (press)                    (✓ 当前)
─────────────
TTS Engine   ▸  edge-tts / mimo-tts                                      (✓ 当前)
TTS Input    ▸  Selection (Ctrl+C) / Clipboard                           (✓ 当前)
─────────────
Toggle Recording
Settings…
Quit
```

各子菜单的当前选项会打勾。菜单与 tooltip 由纯数据模型在状态变化时实时重建，切换引擎/模式无需重启。

---

## 配置

配置文件位置：

| 平台    | 路径                                                      |
| ------- | --------------------------------------------------------- |
| Windows | `%APPDATA%\vox\vox\config\config.toml`                  |
| macOS   | `~/Library/Application Support/com.vox/vox/config.toml` |
| Linux   | `~/.config/vox/config.toml`                             |

```toml
[hotkey]
record_toggle = "Alt+`"
engine_switch = "Alt+Shift+E"
inject_mode_switch = "Alt+Shift+V"
tts_trigger = "Alt+T"

[asr]
primary_engine = "whisper-cpp"          # 本地，免密钥
fallback_engines = ["openai"]

[asr.whisper_cpp]
base_url = "http://127.0.0.1:8080"      # whisper.cpp HTTP 服务

[asr.openai]
base_url = "https://api.openai.com/v1"  # 也可指向本地 OpenAI 兼容服务
api_key = ""
model = "whisper-1"

[asr.mimo]
base_url = "https://token-plan-cn.xiaomimimo.com/v1"
api_key = ""
model = "mimo-v2.5-asr"

[inject]
mode = "keyboard"

[tts]
primary_engine = "edge-tts"             # 免费，无需 API Key
input_mode = "selection"

[tts.edge]
voice = "zh-CN-XiaoxiaoNeural"
rate = "+0%"
volume = "+0%"
pitch = "+0Hz"

[tts.mimo]
model = "mimo-v2.5-tts"
voice = "default"
speed = 1.0
```

---

## 可用引擎

### ASR

| 引擎                                       | 类型                        | 状态                                           |
| ------------------------------------------ | --------------------------- | ---------------------------------------------- |
| **whisper.cpp** (`whisper-cpp`)    | 本地（HTTP 服务）           | ✅ 默认 — 免密钥、无 FFI                      |
| **OpenAI 兼容** (`openai`)         | 云端/本地（REST multipart） | ✅`base_url` 可指向 localhost                |
| **Mimo ASR** (`mimo`)              | 云端（多模态对话）          | ✅ 需 API Key                                  |
| **阿里云 ASR** (`aliyun`)          | 云端（一句话识别）          | ✅ 需 appkey + token                           |
| **Whisper 本地** (`whisper-local`) | 本地（whisper-rs FFI）      | ⚠️ 需`--features whisper-local` + libclang |

### TTS

| 引擎                              | 类型                 | 状态                      |
| --------------------------------- | -------------------- | ------------------------- |
| **Edge TTS** (`edge-tts`) | 云端（免费、免密钥） | ✅ 默认 — 微软 Edge 朗读 |
| **Mimo TTS** (`mimo-tts`) | 云端（神经 TTS）     | ✅ 需 API Key             |

---

## 项目结构

```
vox/
├── Cargo.toml
├── SPEC.md              # 技术规范
├── PLAN.md              # 实施计划
└── src/
    ├── main.rs          # 事件循环、CLI 子命令、引擎装配
    ├── app/             # 状态机 & 全局热键
    ├── asr/             # AsrEngine trait + manager(fallback) + 各引擎
    │                     whisper_cpp / openai / mimo / aliyun / whisper_local
    ├── audio/           # 麦克风采集、WAV 编码、重采样
    ├── config/          # TOML 配置（serde-default 向后兼容）
    ├── inject/          # 文字注入（键盘/剪贴板）+ 剪贴板快照保护
    ├── settings/        # egui 设置窗口（Config 快照的纯视图）
    ├── tray/            # 系统托盘 + 由 MenuModel 构建的菜单
    └── tts/             # TtsEngine trait + manager + 各引擎
                          edge_tts / mimo_tts + rodio 播放
```

---

## 开发

```bash
# 编译
cargo build

# 调试模式运行（显示详细日志）
RUST_LOG=debug cargo run

# 运行测试
cargo test

# 发布编译
cargo build --release

# 调试 CLI 子命令（无需 GUI）：
cargo run -- transcribe <audio.wav>          # 用文件测试 ASR
cargo run -- inject "<文字>" --mode keyboard # 测试文字注入
cargo run -- tts "<文字>" [out.mp3]          # 测试 TTS（写文件并播放）
```

---

## 许可

MIT