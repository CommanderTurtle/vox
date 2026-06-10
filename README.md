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
- 🧠 **Multiple ASR Engines** — Switch between cloud engines (Mimo AI, OpenAI Whisper, Aliyun ASR) and local inference (Whisper.cpp)
- 🗣️ **Multiple TTS Engines** — Cloud neural TTS via Mimo AI, with speed and voice configuration
- 🪶 **Minimal Footprint** — Pure system tray icon, no main window, ~1MB release binary, zero CPU when idle
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
# Run the app
cargo run --release

# The tray icon appears in your system tray.
# Press Alt+` to start recording, say something, press Alt+` again.
# The transcribed text appears at your cursor position.
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

| Action | Default | Description |
|--------|---------|-------------|
| Toggle recording | <kbd>Alt</kbd>+<kbd>`</kbd> | Start / stop microphone input |
| Switch ASR engine | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> | Cycle through available engines |
| Switch inject mode | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> | Toggle keyboard / clipboard injection |
| TTS trigger | <kbd>Alt</kbd>+<kbd>T</kbd> | Read selected text (or clipboard) aloud |

All keybindings are configurable in `config.toml`.

---

## Tray Menu

```
ASR Engine       →  mimo / openai / aliyun / whisper-local
Inject Mode      →  keyboard / clipboard
TTS Engine       →  mimo-tts
TTS Input        →  Selection / Clipboard
Settings         →  Open settings window
Quit             →  Exit vox
```

---

## Configuration

Configuration file location:

| Platform | Path |
|----------|------|
| Windows | `%APPDATA%\vox\vox\config\config.toml` |
| macOS | `~/Library/Application Support/com.vox/vox/config.toml` |
| Linux | `~/.config/vox/config.toml` |

```toml
[hotkey]
record_toggle = "Alt+`"
engine_switch = "Alt+Shift+E"
inject_mode_switch = "Alt+Shift+V"
tts_trigger = "Alt+T"

[asr]
primary_engine = "mimo"
fallback_engines = ["whisper-local", "openai"]

[asr.mimo]
base_url = "https://token-plan-cn.xiaomimimo.com/v1"
api_key = "your-api-key"
model = "mimo-v2.5-asr"

[tts]
primary_engine = "mimo-tts"
input_mode = "selection"

[tts.mimo]
model = "mimo-v2.5-tts"
voice = "default"
speed = 1.0

[inject]
mode = "keyboard"
```

---

## Available Engines

### ASR

| Engine | Type | Status |
|--------|------|--------|
| **Mimo AI ASR** (`mimo`) | Cloud (multimodal chat) | ✅ Working |
| **OpenAI Whisper** (`openai`) | Cloud (REST API) | ✅ Implemented |
| **Aliyun ASR** (`aliyun`) | Cloud (一句话识别) | ✅ Implemented |
| **Whisper Local** (`whisper-local`) | Local (Whisper.cpp) | ⚠️ Requires libclang |

### TTS

| Engine | Type | Status |
|--------|------|--------|
| **Mimo AI TTS** (`mimo-tts`) | Cloud (neural TTS) | ✅ Working |

---

## Project Structure

```
vox/
├── Cargo.toml
├── SPEC.md              # Specification
├── PLAN.md              # Implementation plan
├── TASKS.md             # Task breakdown
└── src/
    ├── main.rs          # Event loop & entry point
    ├── app/             # State machine & hotkeys
    ├── asr/             # ASR engine trait & implementations
    ├── audio/           # Microphone capture & WAV utils
    ├── config/          # TOML configuration management
    ├── inject/          # Text injection (keyboard/clipboard/reader)
    ├── settings/        # Minimal egui settings window
    ├── tray/            # System tray icon & menu
    └── tts/             # TTS engine trait & implementations
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
- 🧠 **多 ASR 引擎** — 云端引擎（小米 Mimo AI、OpenAI Whisper、阿里云 ASR）和本地推理（Whisper.cpp）自由切换
- 🗣️ **多 TTS 引擎** — 云端神经 TTS（小米 Mimo），支持语速和音色配置
- 🪶 **极致轻量** — 纯系统托盘图标，无主窗口，Release 产物约 1MB，空闲时零 CPU 占用
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
# 运行
cargo run --release

# 托盘图标出现后，按 Alt+` 开始录音，说完再按 Alt+` 停止
# 识别的文字自动出现在光标位置
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

| 操作 | 默认按键 | 说明 |
|------|---------|------|
| 启停录音 | <kbd>Alt</kbd>+<kbd>`</kbd> | 开始 / 停止麦克风输入 |
| 切换 ASR 引擎 | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> | 循环切换可用引擎 |
| 切换注入模式 | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> | 键盘模拟 / 剪贴板粘贴 |
| TTS 触发 | <kbd>Alt</kbd>+<kbd>T</kbd> | 朗读选中文字（或剪贴板内容） |

所有快捷键可在 `config.toml` 中自定义。

---

## 托盘菜单

```
ASR Engine       →  mimo / openai / aliyun / whisper-local
Inject Mode      →  keyboard / clipboard
TTS Engine       →  mimo-tts
TTS Input        →  Selection / Clipboard
Settings         →  打开设置窗口
Quit             →  退出 vox
```

---

## 配置

配置文件位置：

| 平台 | 路径 |
|------|------|
| Windows | `%APPDATA%\vox\vox\config\config.toml` |
| macOS | `~/Library/Application Support/com.vox/vox/config.toml` |
| Linux | `~/.config/vox/config.toml` |

```toml
[hotkey]
record_toggle = "Alt+`"
engine_switch = "Alt+Shift+E"
inject_mode_switch = "Alt+Shift+V"
tts_trigger = "Alt+T"

[asr]
primary_engine = "mimo"
fallback_engines = ["whisper-local", "openai"]

[asr.mimo]
base_url = "https://token-plan-cn.xiaomimimo.com/v1"
api_key = "你的 API Key"
model = "mimo-v2.5-asr"

[tts]
primary_engine = "mimo-tts"
input_mode = "selection"

[tts.mimo]
model = "mimo-v2.5-tts"
voice = "default"
speed = 1.0

[inject]
mode = "keyboard"
```

---

## 可用引擎

### ASR

| 引擎 | 类型 | 状态 |
|------|------|------|
| **小米 Mimo ASR** (`mimo`) | 云端（多模态对话） | ✅ 工作中 |
| **OpenAI Whisper** (`openai`) | 云端（REST API） | ✅ 已实现 |
| **阿里云 ASR** (`aliyun`) | 云端（一句话识别） | ✅ 已实现 |
| **Whisper 本地** (`whisper-local`) | 本地（Whisper.cpp） | ⚠️ 需要 libclang |

### TTS

| 引擎 | 类型 | 状态 |
|------|------|------|
| **小米 Mimo TTS** (`mimo-tts`) | 云端（神经 TTS） | ✅ 工作中 |

---

## 项目结构

```
vox/
├── Cargo.toml
├── SPEC.md              # 技术规范
├── PLAN.md              # 实施计划
├── TASKS.md             # 任务拆分
└── src/
    ├── main.rs          # 主循环 & 入口
    ├── app/             # 状态机 & 全局热键
    ├── asr/             # ASR 引擎接口 & 实现
    ├── audio/           # 麦克风采集 & WAV 工具
    ├── config/          # TOML 配置管理
    ├── inject/          # 文字注入（键盘/剪贴板/读选中文字）
    ├── settings/        # 极简 egui 设置窗口
    ├── tray/            # 系统托盘图标 & 菜单
    └── tts/             # TTS 引擎接口 & 实现
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
```

---

## 许可

MIT
