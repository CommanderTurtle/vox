# vox — 技术实施计划

## 1. 架构总览

```
┌──────────────────────────────────────────────────────────────────┐
│  Thread 1 (Main): crossbeam::select! 事件循环                      │
│  ┌──────────────┐  ┌──────────────────┐  ┌────────────────────┐  │
│  │ 托盘事件      │  │ 热键事件           │  │ tokio Runtime      │  │
│  │ TrayEvent    │◄─┤ HotkeyEvent      ├──►│ block_on(ASR/TTS) │  │
│  └──────┬───────┘  └────────┬─────────┘  └────────────────────┘  │
│         │                   │                                     │
│  ┌──────▼───────────────────▼────────────────────────────────┐   │
│  │  AppCtx: ConfigManager / AsrManager / TtsManager / State   │   │
│  └───────────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────────┘
        │                     │
  ┌─────▼─────┐         ┌────▼─────┐
  │ Thread 2  │         │ Thread 3 │
  │ rdev 热键  │         │ cpal 音频 │
  └───────────┘         └──────────┘
```

### 跨线程通信
- 主线程：`crossbeam::channel` 接收 TrayEvent + HotkeyEvent
- 热键线程 → `HotkeyEvent` channel
- 托盘菜单线程 → `TrayEvent` channel
- 音频采集线程：`AtomicBool` 控制启停
- TTS 播放：`std::thread::spawn` 不阻塞主循环

---

## 2. Phase 实际完成情况

### Phase 1: MVP 端到端闭环
| 步骤 | 状态 |
|------|------|
| 1.1 脚手架 + 配置系统 | ✅ |
| 1.2 系统托盘 | ✅ (专用线程 + PeekMessageW) |
| 1.3 全局热键 | ✅ |
| 1.4 音频采集 (cpal) | ✅ |
| 1.5 ASR 引擎 trait + whisper | ✅ (feature-gated) |
| 1.6 键盘注入 (enigo) | ✅ |
| 1.7 主循环组装 | ✅ |

### Phase 2: 云 ASR + 引擎切换
| 步骤 | 状态 |
|------|------|
| 2.1 小米 Mimo ASR | ✅ (chat completions + input_audio) |
| 2.2 OpenAI Whisper API | ✅ |
| 2.3 阿里云 ASR | ✅ |
| 2.4 托盘菜单 + fallback | ✅ |

### Phase 3: TTS + 设置界面
| 步骤 | 状态 |
|------|------|
| 3.1 小米 Mimo TTS | ✅ (base64 PCM → WAV) |
| 3.2 TTS 输入模式 (选中/剪贴板) | ✅ |
| 3.3 音频播放 | ✅ (temp WAV + 系统播放器) |
| 3.4 设置界面 (egui) | ✅ |

### Phase 4: 打包 + 自启动
| 步骤 | 状态 |
|------|------|
| 4.1 Windows 自启动脚本 | ✅ `scripts/install-autostart.bat` |
| 4.2 macOS/Linux 自启动脚本 | ✅ `scripts/install-service.sh` |
| 4.3 打包脚本 | ✅ `scripts/package.sh` |
| 4.4 CLI 调试子命令 | ✅ `vox transcribe` / `vox inject` |

---

## 3. 依赖清单

```toml
[package]
name = "vox"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = "1"            # async runtime
async-trait = "0.1"    # async trait support
cpal = "0.15"          # audio capture
rdev = "0.5"           # global hotkeys
tray-icon = "0.19"     # system tray
enigo = "0.6"          # keyboard simulation
arboard = "3"          # clipboard
reqwest = "0.12"       # HTTP client (cloud ASR/TTS)
serde = "1"            # serialization
serde_json = "1"
toml = "0.8"
directories = "5"      # config paths
hound = "3"            # WAV encode/decode
thiserror = "2"        # error types
base64 = "0.22"        # Mimo TTS audio decode
log = "0.4"
env_logger = "0.11"
crossbeam = "0.8"      # channels
tempfile = "3"         # TTS temp WAV file
egui = "0.34"          # settings UI
eframe = "0.34"
whisper-rs = "0.13"    # optional, local ASR
windows-sys = "0.59"   # Windows only, message pump
```

### 关键风险与缓解

| 风险 | 缓解方案 |
|------|---------|
| whisper-rs 编译失败 | feature-gated；默认只用云引擎 |
| Wayland 热键不工作 | rdev → X11/XWayland fallback |
| Windows 托盘右键无反应 | 专用线程 + PeekMessageW 消息泵 |
| Mimo API 非标准 (chat completions) | 已适配 input_audio 和 TTS 格式 |

---

## 4. 配置参考 (defaults.toml)

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
api_key = ""
model = "mimo-v2.5-asr"

[asr.whisper_local]
model = "tiny"
model_path = ""

[asr.aliyun]
appkey = ""
token = ""

[asr.openai]
api_key = ""
model = "whisper-1"

[inject]
mode = "keyboard"

[tts]
primary_engine = "mimo-tts"
input_mode = "selection"

[tts.mimo]
model = "mimo-v2.5-tts"
voice = "default"
speed = 1.0

[general]
autostart = true
language = "auto"
```

---

## 5. 配置路径

| 平台 | 路径 |
|------|------|
| Windows | `%APPDATA%\vox\vox\config\config.toml` |
| macOS | `~/Library/Application Support/com.vox/vox/config.toml` |
| Linux | `~/.config/vox/config.toml` |
