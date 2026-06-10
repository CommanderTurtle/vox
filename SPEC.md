# vox — 语音输入输出增强工具 · 技术规范

## 1. Objective

### What
**Voice Boost** 是一个跨平台的系统托盘应用，为命令行 AI Agent 工具（如 opencode、oh-my-pi、reasonix 等）及任何桌面文本输入区域添加**全局语音输入**能力，同时支持**选中文档朗读（TTS）**。

### Why
现有的 CLI Agent 工具缺乏语音输入支持，打字输入效率低、场景受限。ASR Boost 不依赖 Agent 本身集成，而是作为系统级增强层存在，对所有应用透明生效。

### Who
- 重度 CLI Agent 用户
- 需要在终端/编辑器中快速录入口头想法或指令的开发者
- 多模态 prompt 输入场景

### Success Criteria
- 全局热键启停录音 → 文字 **≤1.5 秒**内出现在目标输入位置（本地 Whisper）
- 软件完全无窗口运行，仅系统托盘图标交互
- 支持 3 种 ASR 引擎并可热切换（本地 Whisper / 阿里云 ASR / OpenAI Whisper API）
- 插件式架构，新增 ASR 引擎无需改核心逻辑
- 支持 2 种文字注入模式（模拟键盘 / 剪贴板粘贴）并可切换
- 非录音时零 CPU/GPU 占用（<5MB 常驻内存）
- 在 Windows / macOS / Linux 上编译通过并正常工作

---

## 2. Commands

### Build & Run

```bash
# 开发调试
cargo run

# 指定 ASR 引擎启动（覆盖配置）
cargo run -- --asr-engine whisper-local

# 生产构建
cargo build --release
./target/release/asr-boost

# 仅识别音频文件（测试/调试模式）
./asr-boost transcribe /path/to/audio.wav --engine whisper-local

# 安装为系统服务/自启动（各平台不同）
# Windows: sc create / 注册表 Run 键
# macOS: LaunchAgents
# Linux: systemd user service / .desktop autostart
```

### Tray Icon & Interaction
- 左键单击 → 切换录音（开始/停止）
- 右键单击 → 弹出菜单：[引擎切换]、[注入模式切换]、[设置]、[退出]
- 设置窗口：极小设置界面（热键绑定、引擎配置、模型路径等）

### 全局热键（可配置）

| 功能 | 默认快捷键 |
|------|-----------|
| 开始/停止录音 | `Alt+` |
| 切换注入模式 | `Alt+Shift+V` |
| 切换 ASR 引擎 | `Alt+Shift+E` |

> 注：反引号键在终端区顺手且不易冲突。

---

## 3. Project Structure

```
asr-boost/
├── Cargo.toml
├── Cargo.lock
├── SPEC.md                      # 本规范
├── assets/
│   ├── icon.png                 # 托盘图标（16/32/64 多尺寸）
│   └── icon.ico                 # Windows 图标
├── src/
│   ├── main.rs                  # 入口：初始化日志、托盘、引擎、热键
│   ├── app/
│   │   ├── mod.rs
│   │   ├── state.rs             # 全局应用状态（当前引擎、模式、录音状态）
│   │   └── hotkey.rs            # 全局热键注册/反注册
│   ├── tray/
│   │   ├── mod.rs
│   │   └── menu.rs              # 系统托盘菜单构建与事件循环
│   ├── audio/
│   │   ├── mod.rs
│   │   ├── capture.rs           # 麦克风录音 PCM 数据采集
│   │   └── utils.rs             # WAV 编码/临时文件管理/电平检测
│   ├── asr/
│   │   ├── mod.rs               # AsrEngine trait 定义
│   │   ├── whisper_local.rs     # whisper.cpp FFI 封装
│   │   ├── aliyun_asr.rs        # 阿里云实时语音识别
│   │   └── openai_asr.rs        # OpenAI Whisper API (REST)
│   ├── inject/
│   │   ├── mod.rs               # TextInjector trait 定义
│   │   ├── keyboard.rs          # 模拟键盘输入（SendInput / uinput）
│   │   └── clipboard.rs         # 剪贴板写入 + Ctrl+V 模拟粘贴
│   └── config/
│       ├── mod.rs
│       └── defaults.toml        # 默认配置文件（嵌入成常量）
├── tests/
│   ├── integration_test.rs
│   └── fixtures/
│       └── hello.wav            # 1 秒"你好"测试音频
└── scripts/
    ├── build-whisper.sh         # 下载/编译 whisper.cpp
    └── install-service.sh       # 安装自启动（按平台分支）
```

---

## 4. Code Style

### 核心架构：异步事件驱动

```rust
// src/asr/mod.rs — ASR 引擎接口
#[async_trait]
pub trait AsrEngine: Send + Sync {
    /// 引擎唯一标识
    fn name(&self) -> &'static str;

    /// 同步识别（一次录音文件）
    async fn transcribe(&self, audio: &[u8]) -> Result<String, AsrError>;

    /// 可选：实时流式识别（返回一个 Stream）
    fn stream_transcribe(
        &self,
        stream: Pin<Box<dyn Stream<Item = Vec<i16>> + Send>>,
    ) -> Pin<Box<dyn Stream<Item = Result<String, AsrError>> + Send>>
    where
        Self: Sized;
}
```

```rust
// src/inject/mod.rs — 文本注入接口
#[async_trait]
pub trait TextInjector: Send + Sync {
    fn inject(&self, text: &str) -> Result<(), InjectError>;
}
```

### 状态机（录音生命周期）

```rust
#[derive(Clone, Copy, PartialEq)]
pub enum AppState {
    Idle,           // 等待录音
    Recording,      // 正在录音
    Transcribing,   // 正在识别（短暂，可忽略）
}
```

### 命名约定
- 类型：`PascalCase`
- 变量/函数/模块：`snake_case`
- 常量/环境变量：`SCREAMING_SNAKE_CASE`
- 模块文件名与 `mod.rs` 中的 `pub mod` 同名

### 错误处理
```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("ASR recognition failed: {0}")]
    Asr(#[from] AsrError),
    #[error("Audio capture failed: {0}")]
    Audio(String),
    #[error("Text injection failed: {0}")]
    Inject(#[from] InjectError),
    #[error("Config error: {0}")]
    Config(#[from] ConfigError),
}
```

---

## 5. Testing Strategy

| 层级 | 位置 | 覆盖目标 |
|------|------|----------|
| 单元测试 | `src/**/*.rs` 内 `#[cfg(test)] mod tests` | 状态机转移、配置解析、音频编码/解码 |
| 集成测试 | `tests/integration_test.rs` | ASR 引擎 mock 调用、文本注入 mock 调用、全链路（mock 采集→mock ASR→mock 注入） |
| 手动/端到端 | 本地运行 + 录音测试 | 热键实际注册、真实麦克风采集、真实 ASR 调用 |

暂不追求高覆盖率的 CI 自动测试（平台专有 API 模拟困难），但**核心逻辑通路必须有 mock 测试**。

### Mock 策略
```rust
// 使用 mockall crate
#[cfg(test)]
mock! {
    pub AsrEngine {
        fn name(&self) -> &'static str;
        async fn transcribe(&self, audio: &[u8]) -> Result<String, AsrError>;
    }
}
```

---

## 6. Boundaries

### Always Do
- 热键释放后立即停止录音并触发 ASR
- 所有配置持久化到配置文件（TOML），支持运行时热重载
- ASR 引擎失败时自动 fallback（按用户预设的优先级列表）
- 录音时显示视觉反馈（托盘图标变化 / 可选的浮动指示器）
- 各平台原生安装方式打包

### Ask First (Before Implementing)
- 是否添加实时流式识别（边录边出文字）—— 会增加复杂度
- 是否支持多语言自动检测
- 是否添加自定义音频设备选择
- 是否录制系统音频 + 麦克风混音（会议场景）

### Never Do
- ❌ 内置 GUI 主窗口 —— 设置最多一个 tiny 弹窗
- ❌ 收集用户录音/隐私数据
- ❌ 依赖特定 CLI Agent 的 API/plugin —— 始终是全局注入
- ❌ 录音时阻塞 UI —— 必须异步
- ❌ 引入需要巨大运行时（Python/Node）的依赖

---

## 附录 A: 技术栈详细说明

| 模块 | 方案 | 理由 |
|------|------|------|
| 跨平台音频捕获 | `cpal` crate | Rust 生态最成熟的音频输入库 |
| 全局热键 | `rdev`（跨平台） | 捕捉热键事件 |
| 系统托盘 | `tray-icon` crate | 原生的跨平台托盘 |
| 本地 ASR | `whisper.cpp` Rust binding（`whisper-rs` 或 FFI） | 离线、小体积、开源 |
| 阿里云 ASR | REST API (HTTP) over `reqwest` | 标准 REST 接口 |
| OpenAI Whisper API | REST API over `reqwest` | 标准 REST 接口 |
| 键盘模拟 | Windows: `windows-rs` SendInput / macOS: CoreGraphics / Linux: `uinput` 或 `enigo` | 平台专有但精确 |
| 剪贴板 | `arboard` crate | 跨平台剪贴板读写 |
| 配置 | `serde` + `toml` | 结构化、人类可读 |
| 异步运行时 | `tokio` | Rust 标准异步运行时 |
| 串行化 | `serde` / `serde_json` | 序列化/反序列化标准 |

## 附录 B: 工作流

```
用户按下录音热键
    → capture.rs 开始采集麦克风 PCM（cpal）
    → 托盘图标变为红色/录音状态
用户再次按下 / 松键
    → capture.rs 停止采集 → 输出 &[u8] (WAV bytes)
    → state.rs 状态 → Transcribing
    → 当前 AsrEngine::transcribe(audio_bytes) → Result<String>
    → 当前 TextInjector::inject(text)
    → 状态回到 Idle
```
