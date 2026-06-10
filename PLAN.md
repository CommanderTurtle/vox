# vox — 技术实施计划

## 1. 架构总览

```
┌──────────────────────────────────────────────────────────────┐
│  Thread 1 (Main): 系统托盘事件循环                              │
│  ┌─────────────┐  ┌─────────────────┐  ┌─────────────────┐   │
│  │ tray-icon   │  │ 状态管理            │  │ tokio handle    │   │
│  │ event loop  │◄─┤ Arc<RwLock<St>>  ├──►│ (spawn ASR/注入) │   │
│  └──────┬──────┘  └─────────────────┘  └────────┬────────┘   │
│         │                                        │           │
│  ┌──────▼──────┐   ┌─────────────────┐          │           │
│  │ 托盘菜单      │   │ 图标更新（Idle/Rec）│         │           │
│  └──────┬──────┘   └─────────────────┘          │           │
│         │                                        │           │
│  ┌──────▼───────────────────────────────────────▼───────┐   │
│  │  跨线程通信: mpsc channel                            │   │
│  └─────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
                           │
               ┌───────────┼─────────────┐
               ▼           ▼              ▼
┌──────────────────┐ ┌──────────┐ ┌──────────────┐
│ Thread 2: rdev   │ │ Thread 3 │ │ Tokio Tasks   │
│ 全局热键监听       │ │ 音频采集   │ │ ASR / 注入    │
│ (callback→ch)    │ │ (cpal)   │ │ (async)       │
└──────────────────┘ └──────────┘ └──────────────┘
```

### 跨线程通信模型

```rust
/// 主线程接收的事件
enum MainEvent {
    /// 热键按下/抬起
    HotkeyPress,
    HotkeyRelease,
    /// 音频采集完成 → PCM Vec<i16>
    AudioCaptured(Vec<i16>),
    /// ASR 识别完成
    TranscriptionResult(String),
    /// ASR 识别失败
    TranscriptionError(AsrError),
    /// 配置变更（托盘菜单触发的设置修改后）
    ConfigChanged(Box<Config>),
    /// 程序退出
    Quit,
}
```

**关键设计原则：**
- 热键监听线程是唯一"始终运行"的后台线程，其他都在需要时启动/停止
- 音频采集在一个独立线程跑，用 flag 控制开始/停止
- ASR 和注入跑在 tokio 上（非阻塞，可并发）
- 主线程通过 channel 接收事件，驱动状态转移和 UI 更新

---

## 2. 分阶段实施路线

### Phase 1: MVP — 端到端闭环（本地 Whisper 优先）

```
目标：能录 → 能识别 → 能打字到光标位置
核心模块：配置 + 托盘 + 热键 + 音频采集 + 本地 ASR + 键盘注入
```

| 步骤 | 内容 | 文件数 | 关键依赖/风险 |
|------|------|--------|-------------|
| 1.1 | 项目脚手架 + Cargo.toml + 配置系统 | 4 | 选型正确性 |
| 1.2 | 系统托盘 + 退出功能 | 3 | tray-icon 跨平台兼容 |
| 1.3 | 全局热键监听 | 2 | rdev 需管理员/辅助权限 |
| 1.4 | 录音采集（cpal）→ WAV 编码 | 2 | 设备枚举/权限 |
| 1.5 | ASR 引擎 trait + whisper.cpp binding | 3 | whisper.cpp 编译最难 |
| 1.6 | 键盘模拟注入（enigo） | 2 | 跨平台输入法兼容 |
| 1.7 | 主循环组装：热键→录音→识别→注入 | 1 | 多线程协调 |

**MVP checkpoint: 按一次热键→录音→自动停止→文字出现在记事本里**

### Phase 2: 云 ASR + 引擎切换

| 步骤 | 内容 | 文件数 |
|------|------|--------|
| 2.1 | 阿里云实时 ASR（REST） | 2 |
| 2.2 | OpenAI Whisper API | 1 |
| 2.3 | 托盘菜单引擎切换 + 配置文件热重载 | 3 |
| 2.4 | 引擎 fallback 链 | 1 |

### Phase 3: 注入模式 + 设置界面

| 步骤 | 内容 | 文件数 |
|------|------|--------|
| 3.1 | 剪贴板模式（arboard + enigo Ctrl+V） | 1 |
| 3.2 | 注入模式切换（托盘菜单） | 1 |
| 3.3 | 极简设置界面（winit 弹窗） | 3 |
| 3.4 | 自启动安装脚本 | 2 |

### Phase 4: 打磨 + 打包

| 步骤 | 内容 |
|------|------|
| 4.1 | 托盘图标录音状态动画 |
| 4.2 | 各平台打包（msi / dmg / AppImage） |
| 4.3 | 错误提示（toast 通知） |
| 4.4 | 性能基准测试 |

---

## 3. 依赖清单与风险评估

### Cargo.toml 核心依赖

```toml
[package]
name = "asr-boost"
version = "0.1.0"
edition = "2021"

[dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# Audio capture
cpal = "0.15"

# Global hotkeys
rdev = "0.5"

# System tray
tray-icon = "0.18"

# Keyboard simulation (enigo 0.2 current)
enigo = "0.2"

# Clipboard
arboard = "3"

# HTTP client
reqwest = { version = "0.12", features = ["json"] }

# Config
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
directories = "5"    # XDG/compatible config dirs

# ASR - Local Whisper
whisper-rs = "0.13"

# Audio format
hound = "3"          # WAV encode/decode

# Error handling
thiserror = "2"
anyhow = "1"

# Logging
log = "0.4"
env_logger = "0.11"

# Concurrency
crossbeam = "0.8"    # For audio capture atomic stop

# Testing (dev-dependencies)
[dev-dependencies]
mockall = "0.13"
tempfile = "3"
```

### 关键风险与缓解

| 风险 | 概率 | 影响 | 缓解 |
|------|------|------|------|
| **whisper-rs 编译失败**（FFI 绑定、C++ 编译器等） | 中 | 高 | MVP 阶段直接屏蔽 whisper-local，先做云端 ASR 打通全链路；whisper-local 作为独立增量 |
| **全局热键在 Wayland 下不工作** | 高（Linux） | 中 | rdev 在 Wayland 确实受限。方案：检测运行环境，fallback 到 X11/XWayland；文档注明 |
| **macOS 辅助功能权限弹窗** | 确定 | 中 | 首次启动引导用户授权；代码中检测权限状态并提示 |
| **cpal 默认设备不工作或无声** | 低 | 中 | 支持设备枚举 + 用户配置默认设备 |
| **rdev 热键在 terminal 中被应用捕获** | 中 | 中 | 默认用 Alt+反引号这种终端不常用的组合；支持用户自选快捷键 |
| **输入法干扰（中文输入法下键盘模拟）** | 中 | 中 | 注入前先切换到英文输入法（复杂）；或始终用剪贴板模式作为 fallback |

### 妥协方案（何时降级）

- 如果 whisper-rs 编译无法解决 → **MVP 用 OpenAI Whisper API** 作为唯一引擎，本地引擎后续再补
- 如果 tray-icon + rdev 事件循环冲突 → **把 rdev 跑在独立线程**（已在计划中）
- 如果跨平台设置窗口太复杂 → **设置用 TOML 文件 + 默认编辑器打开**，跳过 GUI 弹窗

---

## 4. 模块详细设计

### 4.1 配置系统 (`src/config/`)

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub asr: AsrConfig,
    pub inject: InjectConfig,
    pub general: GeneralConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HotkeyConfig {
    pub record_toggle: String,        // "Alt+`"
    pub engine_switch: String,        // "Alt+Shift+E"
    pub inject_mode_switch: String,   // "Alt+Shift+V"
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AsrConfig {
    pub primary_engine: String,       // "whisper-local" | "aliyun" | "openai"
    pub fallback_engines: Vec<String>,
    // whisper-local
    pub whisper_model_path: Option<String>,  // None → 自动下载 tiny model
    // aliyun
    pub aliyun_appkey: Option<String>,
    pub aliyun_token: Option<String>,
    // openai
    pub openai_api_key: Option<String>,
    pub openai_model: Option<String>,        // "whisper-1"
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum InjectMode {
    Keyboard,    // 模拟击键
    Clipboard,   // 剪贴板粘贴
}

pub struct ConfigManager {
    path: PathBuf,
    config: Arc<RwLock<Config>>,
    watcher: Option<Notify>,   // 文件变更通知
}
```

TOML 配置文件路径：
- Linux: `~/.config/asr-boost/config.toml`
- macOS: `~/Library/Application Support/com.asr-boost/config.toml`
- Windows: `%APPDATA%\asr-boost\config.toml`

### 4.2 音频采集 (`src/audio/`)

```rust
pub struct AudioCapture {
    device: cpal::Device,
    config: cpal::SupportedStreamConfig,
    stop_flag: Arc<AtomicBool>,
}

impl AudioCapture {
    /// 开始采集，返回一个 receiver 接收帧数据
    pub fn start(&self) -> crossbeam::Receiver<Vec<i16>> {
        // 用 cpal 的 input_stream 读取 PCM i16
        // 每次回调 push 到 crossbeam channel
        // stop_flag 控制何时停止
    }

    /// 停止采集，返回完整 PCM 数据
    pub fn stop(&self) -> Vec<i16> {
        self.stop_flag.store(true, Ordering::Relaxed);
        // 从 channel 收集所有剩余数据
    }
}
```

音频格式固定：**16kHz, 单声道, i16 PCM**（whisper 和 ASR API 都支持）。

### 4.3 ASR 引擎 (`src/asr/`)

```rust
#[async_trait]
pub trait AsrEngine: Send + Sync {
    fn name(&self) -> &'static str;
    async fn transcribe(&self, audio: &[u8]) -> Result<String, AsrError>;
}

pub struct AsrManager {
    engines: HashMap<String, Box<dyn AsrEngine>>,
    active: String,
    fallback_order: Vec<String>,
}

impl AsrManager {
    /// 按活跃引擎识别，失败后按 fallback 顺序重试
    pub async fn transcribe_with_fallback(&self, audio: &[u8]) -> Result<String> {
        // 1. try active → ok? return
        // 2. try fallback[0] → ok? update active & return
        // 3. try fallback[1] ...
        // 4. all failed → return last error
    }
}
```

### 4.4 文字注入 (`src/inject/`)

```rust
#[async_trait]
pub trait TextInjector: Send + Sync {
    fn inject(&self, text: &str) -> Result<(), InjectError>;
}

pub struct KeyboardInjector {
    enigo: Enigo,
}

pub struct ClipboardInjector {
    enigo: Enigo,    // 用于模拟 Ctrl+V
}

// ClipboardInjector 流程：
// 1. 保存当前剪贴板内容
// 2. arboard 写入识别文本
// 3. enigo 模拟 Ctrl+V
// 4. 恢复剪贴板内容
// 5. 延迟 100ms 恢复以避免竞态
```

### 4.5 主循环 (`src/main.rs`)

```
初始化:
  1. 加载配置
  2. 初始化 AsrManager (注册所有引擎)
  3. 初始化 Injector
  4. 创建托盘 (tray-icon)
  5. 启动热键监听线程 (rdev)
  6. 进入 tray-icon 事件循环

热键事件:
  ┌─ "切换录音"按下 ─────────────────────────┐
  │  if state == Idle:
  │    state = Recording
  │    更新托盘图标 (红点)
  │    启动音频采集线程
  │  else if state == Recording:
  │    state = Transcribing
  │    更新托盘图标 (转圈/等待)
  │    停止音频采集 → 得到 PCM Vec<i16>
  │    spawn tokio task:
  │      → WAV 编码 (hound)
  │      → AsrManager::transcribe_with_fallback(wav_bytes)
  │      → match result:
  │          Ok(text) → injector.inject(text)
  │          Err(e)   → log error + toast 通知
  │      → state = Idle
  │      → 更新托盘图标 (Idle)
  └──────────────────────────────────────────┘
```

---

## 5. 并行策略

| 阶段 | 任务 | 可并行 |
|------|------|--------|
| MVP 1.1 | 脚手架 + 配置 | — |
| MVP 1.2 | 系统托盘 | — |
| MVP 1.3 | 全局热键 | — |
| MVP 1.4 | 音频采集 | 可与 1.2/1.3 同步开发 |
| MVP 1.5 | ASR 引擎 trait + whisper-local | 可与 1.2/1.3/1.4 同步开发 |
| MVP 1.6 | 键盘注入 | 可与 1.2/1.3/1.4 同步开发 |
| MVP 1.7 | 主循环组装 | **依赖前面全部** |

最佳并行化：**两个人同时开发**：
- A：1.2(托盘) + 1.3(热键) + 1.4(音频) + 1.6(注入) → A 不需要 ASR 知识
- B：1.5(ASR trait + whisper binding) → B 需要 ASR 知识
- 合并后 A+B 一起做 1.7

---

## 6. 配置参考 (defaults.toml)

```toml
[hotkey]
record_toggle = "Alt+`"
engine_switch = "Alt+Shift+E"
inject_mode_switch = "Alt+Shift+V"

[asr]
primary_engine = "whisper-local"
fallback_engines = ["openai"]

[asr.whisper_local]
model = "tiny"           # tiny|base|small|medium|large
model_path = ""

[asr.aliyun]
appkey = ""
token = ""

[asr.openai]
api_key = ""
model = "whisper-1"

[inject]
mode = "keyboard"        # keyboard|clipboard

[general]
autostart = true
language = "auto"        # auto|zh|en
show_level_indicator = true
```
