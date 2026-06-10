# vox — 语音 I/O 增强工具 · 技术规范

## 1. Objective

### What
**vox** 是一个跨平台的系统托盘应用，为命令行 AI Agent 工具（如 opencode、oh-my-pi、reasonix 等）及任何桌面文本输入区域添加**全局语音输入（ASR）**和**文字转语音（TTS）**能力。

### Why
现有的 CLI Agent 工具缺乏语音 I/O 支持，打字输入效率低、场景受限。vox 不依赖 Agent 本身集成，而是作为系统级增强层存在，对所有应用透明生效。

### Who
- 重度 CLI Agent 用户
- 需要在终端/编辑器中快速录入口头想法或指令的开发者
- 需要文字朗读的多模态场景

### Success Criteria
- 全局热键启停录音 → 文字 **≤3 秒**内出现在目标输入位置（云端 ASR）
- TTS 热键朗读选中文字或剪贴板内容
- 软件完全无窗口运行，仅系统托盘图标交互
- 支持多 ASR 引擎并可热切换（Mimo / OpenAI / 阿里云 / 本地 Whisper）
- 支持 TTS 引擎并可热切换
- 插件式架构，新增引擎无需改核心逻辑
- 支持 2 种文字注入模式（模拟键盘 / 剪贴板粘贴）并可切换
- 非录音时零 CPU/GPU 占用（<5MB 常驻内存）
- 在 Windows / macOS / Linux 上编译通过并正常工作

---

## 2. Commands

### Build & Run

```bash
# 开发调试
cargo run

# 生产构建
cargo build --release
./target/release/vox

# 识别音频文件（调试模式）
cargo run -- transcribe /path/to/audio.wav

# 注入文字到光标（调试模式）
cargo run -- inject "你好世界" --mode keyboard

# 安装为系统服务/自启动
# Windows: scripts\install-autostart.bat
# macOS:   bash scripts/install-service.sh
# Linux:   bash scripts/install-service.sh

# 打包分发
bash scripts/package.sh
```

### Tray Icon & Interaction
- 左键单击 → 切换录音（开始/停止）
- 右键单击 → 弹出菜单：[ASR 引擎]、[注入模式]、[TTS 引擎]、[TTS 输入模式]、[设置]、[退出]
- 设置窗口：极小设置界面（热键绑定、引擎配置、API Key 等）

### 全局热键（可配置）

| 功能 | 默认快捷键 |
|------|-----------|
| 开始/停止录音 | <kbd>Alt</kbd>+<kbd>`</kbd> |
| 切换注入模式 | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>V</kbd> |
| 切换 ASR 引擎 | <kbd>Alt</kbd>+<kbd>Shift</kbd>+<kbd>E</kbd> |
| TTS 触发 | <kbd>Alt</kbd>+<kbd>T</kbd> |

---

## 3. Project Structure

```
vox/
├── Cargo.toml
├── Cargo.lock
├── README.md
├── SPEC.md                      # 技术规范
├── PLAN.md                      # 实施计划
├── TASKS.md                     # 任务拆解
├── AGENTS.md                    # AI 助手记忆
├── scripts/
│   ├── build-whisper.sh         # 下载/编译 whisper.cpp
│   ├── install-service.sh       # macOS/Linux 自启动
│   ├── install-autostart.bat    # Windows 自启动
│   └── package.sh               # 打包分发脚本
└── src/
    ├── main.rs                  # 入口 + CLI 子命令 + 事件循环
    ├── app/
    │   ├── mod.rs
    │   ├── state.rs             # 状态机 (Idle/Recording/Transcribing)
    │   └── hotkey.rs            # 全局热键解析与监听
    ├── tray/
    │   └── mod.rs               # 系统托盘 + 右键菜单 + Windows 消息泵
    ├── audio/
    │   ├── mod.rs
    │   ├── capture.rs           # cpal 麦克风 PCM 采集
    │   └── utils.rs             # WAV 编码/RMS 电平
    ├── asr/
    │   ├── mod.rs               # AsrEngine trait + AsrManager + fallback
    │   ├── whisper_local.rs     # whisper.cpp FFI (feature-gated)
    │   ├── mimo_asr.rs          # 小米 Mimo 多模态 API
    │   ├── aliyun_asr.rs        # 阿里云一句话识别 REST
    │   └── openai_asr.rs        # OpenAI Whisper API
    ├── inject/
    │   ├── mod.rs               # InjectMode + inject_text()
    │   ├── keyboard.rs          # enigo 键盘模拟
    │   ├── clipboard.rs         # 剪贴板写入 + Ctrl+V
    │   └── text_reader.rs       # Ctrl+C 模拟读选中文字
    ├── tts/
    │   ├── mod.rs               # TtsEngine trait + TtsManager + TtsInputMode
    │   ├── mimo_tts.rs          # 小米 Mimo TTS (base64 PCM → WAV)
    │   └── playback.rs          # temp WAV → 系统播放器
    ├── config/
    │   ├── mod.rs               # ConfigManager (TOML)
    │   └── defaults.toml        # 默认配置（编译时嵌入）
    └── settings/
        └── mod.rs               # egui 设置窗口
```

---

## 4. Code Style

### 核心架构：异步事件驱动

```rust
// ASR 引擎接口
#[async_trait]
pub trait AsrEngine: Send + Sync {
    fn name(&self) -> &'static str;
    async fn transcribe(&self, audio: &[u8]) -> Result<String, AsrError>;
}

// TTS 引擎接口
#[async_trait]
pub trait TtsEngine: Send + Sync {
    fn name(&self) -> &'static str;
    async fn synthesize(&self, text: &str) -> Result<Vec<u8>, TtsError>;
}
```

### 状态机

```rust
#[derive(Clone, Copy, PartialEq)]
pub enum AppState {
    Idle,           // 等待录音
    Recording,      // 正在录音
    Transcribing,   // 正在识别
}
```

### 命名约定
- 类型：`PascalCase`
- 变量/函数/模块：`snake_case`
- 常量：`SCREAMING_SNAKE_CASE`

### 错误处理
```rust
#[derive(Debug, thiserror::Error)]
pub enum AsrError { /* EngineError / NoEngineAvailable / EngineNotFound / AudioFormat */ }
#[derive(Debug, thiserror::Error)]
pub enum TtsError { /* EngineError / Playback */ }
#[derive(Debug, thiserror::Error)]
pub enum InjectError { /* Keyboard / Clipboard */ }
```

---

## 5. Testing Strategy

| 层级 | 位置 | 覆盖目标 |
|------|------|----------|
| 单元测试 | `src/**/*.rs` 内 `#[cfg(test)]` | 配置解析、音频编码、热键解析、ASR manager |
| 手动端到端 | 本地运行 | 热键、麦克风、真实 ASR/TTS API 调用 |

**核心逻辑通路有 mock 测试**（`mockall`）。

---

## 6. Boundaries

### Always Do
- 热键触发后立即停止录音并启动 ASR
- 配置持久化到 TOML 文件，`#[serde(default)]` 兼容旧配置
- ASR 引擎失败时自动 fallback
- 录音时托盘 tooltip 变化
- 各平台原生安装方式

### Never Do
- ❌ 内置 GUI 主窗口 —— 设置最多一个 tiny 弹窗
- ❌ 收集用户录音/隐私数据
- ❌ 依赖特定 CLI Agent 的 API/plugin —— 始终全局注入
- ❌ 录音时阻塞 UI
- ❌ Python/Node 等大运行时依赖

---

## 附录 A: 技术栈

| 模块 | 方案 |
|------|------|
| 音频捕获 | `cpal` |
| 全局热键 | `rdev` |
| 系统托盘 | `tray-icon` + Windows `PeekMessageW` |
| 云 ASR/TTS | `reqwest` + `serde_json` |
| 本地 ASR | `whisper-rs` (feature-gated) |
| 键盘模拟 | `enigo` |
| 剪贴板 | `arboard` |
| 配置 | `serde` + `toml` + `directories` |
| 设置 UI | `egui` + `eframe` |
| 异步 | `tokio` + `async-trait` |
| 音频格式 | `hound` (WAV) |
| 跨线程 | `crossbeam` |

## 附录 B: 工作流

```
ASR 流程:
  Alt+` 按下 → capture.start()
  → 托盘变为 Recording
  Alt+` 松开 → capture.stop() → PCM → WAV
  → AsrEngine::transcribe(wav) → text
  → inject_text(text) → 光标处出现文字
  → 托盘回到 Idle

TTS 流程:
  选中文字 → Alt+T 按下
  → text_reader::read_selected_text()  (或 read_clipboard_text)
  → TtsEngine::synthesize(text) → WAV bytes
  → play_wav_async(wav) → 系统播放器发声
```
