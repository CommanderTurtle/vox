# vox — 实施任务列表

> 按顺序逐一执行。每个任务完成后必须验证通过才能进入下一个。

---

## Task 1: 项目脚手架 + 配置系统

**涉及文件 (4):**
- `Cargo.toml`
- `src/main.rs`（骨架）
- `src/config/mod.rs`
- `src/config/defaults.toml`

**验收标准:**
- `cargo build` 成功编译
- `cargo run` 启动进程后即刻退出（还不做托盘，但编译通过）
- `ConfigManager` 能从默认配置初始化、读取/写入 TOML 文件
- 配置文件路径按平台标准（directories crate）

**验证步骤:**
```bash
cargo build
cargo run
# 检查 ~/.config/asr-boost/config.toml 或等效路径是否生成
```

---

## Task 2: 系统托盘 + 基础状态管理

**涉及文件 (4-5):**
- `src/tray/mod.rs`
- `src/tray/menu.rs`
- `src/app/state.rs`
- `src/main.rs`（更新：集成托盘，事件循环保持运行）

**验收标准:**
- 启动后系统托盘出现图标
- 右键菜单包含：[录音切换(禁用)]、[引擎 > 子菜单]、[注入模式 > 子菜单]、[设置]、[退出]
- 左键点击暂无功能（后续实现）
- 点击"退出"能干净退出进程
- 状态机 `AppState` 定义及初始值为 `Idle`

**验证步骤:**
```bash
cargo run
# 观察托盘图标出现
# 右键点击菜单 → 退出 → 进程退出
```

---

## Task 3: 全局热键监听

**涉及文件 (2):**
- `src/app/hotkey.rs`
- `src/main.rs`（更新：启动热键监听线程）

**验收标准:**
- 在独立线程运行 `rdev` 全局事件监听
- 监听 `Alt+``（默认配置）热键组合
- 按下时发送 `MainEvent::HotkeyPress` 到主线程 channel
- 释放时发送 `MainEvent::HotkeyRelease` 到主线程 channel
- 主线程能收到事件并打印日志
- 热键字符串解析成 `rdev::Key` 组合（支持 `Alt+`/`Ctrl+`/`Shift+` 前缀）

**验证步骤:**
```bash
cargo run
# 按下 Alt+` → 终端日志显示 "HotkeyPress"
# 释放 → 终端日志显示 "HotkeyRelease"
# 退出验证
```

---

## Task 4: 音频采集模块

**涉及文件 (3):**
- `src/audio/mod.rs`
- `src/audio/capture.rs`
- `src/audio/utils.rs`

**验收标准:**
- 调用 `AudioCapture::start()` 开始从默认麦克风采集 PCM i16 数据
- 调用 `AudioCapture::stop()` 停止采集并返回 `Vec<i16>`
- 支持 `AtomicBool` 标记停止（非阻塞）
- `utils.rs` 中的 `pcm_to_wav()` 函数将 PCM 编码为 WAV 格式（16kHz, 单声道, i16）
- 写入 WAV 文件后能用 ffprobe/播放器验证格式正确

**验证步骤:**
```bash
cargo run -- transcribe  # debug mode: 录音3秒 → 输出 test.wav
# 检查 test.wav: 16kHz, mono, i16 PCM
# 播放听听是不是自己的声音
```

---

## Task 5: ASR 引擎模块 + 本地 Whisper 实现

**涉及文件 (3):**
- `src/asr/mod.rs`（`AsrEngine` trait + `AsrManager`）
- `src/asr/whisper_local.rs`
- `src/asr/errors.rs`

**验收标准:**
- `AsrEngine` trait 定义完整（`name()`、`transcribe()`）
- `AsrManager` 支持注册引擎、按名称切换、fallback 链
- `WhisperLocal` 实现：加载 whisper.cpp 模型 → 识别 WAV 音频 → 返回文字
- 测试音频（say "你好"）识别结果包含"你好"
- `whisper-rs` FFI 编译通过（可能需要先编译 whisper.cpp）

**如果 whisper-rs 编译失败** → 提供 `FakeAsrEngine` 作为备选（返回固定字符串），跳过本地引擎，后续再解决。

**验证步骤:**
```bash
cargo test --test integration_test  # mock ASR 测试
cargo run -- transcribe test.wav --engine whisper-local
# 输出: "你好" 或类似结果
```

---

## Task 6: 文字注入模块

**涉及文件 (3):**
- `src/inject/mod.rs`
- `src/inject/keyboard.rs`
- `src/inject/clipboard.rs`

**验收标准:**
- `TextInjector` trait 定义：`fn inject(&self, text: &str) -> Result<()>`
- `KeyboardInjector`：模拟逐字键盘输入（使用 `enigo`）
- `ClipboardInjector`：保存剪贴板 → 写入文本 → Ctrl+V → 恢复剪贴板
- 两种模式可在运行时切换

**验证步骤:**
```bash
cargo test --test integration_test  # mock injector 测试
cargo run -- inject "hello world" --mode keyboard
# 打开记事本 → 等待 3 秒 → "hello world" 出现在记事本
cargo run -- inject "hello world" --mode clipboard
# 同上，但通过剪贴板粘贴
```

---

## Task 7: 主循环组装 — 端到端闭环

**涉及文件 (1):**
- `src/main.rs`（完整实现）

**依赖: Task 1~6 全部完成**

**验收标准:**
- 启动后：托盘图标显示 Idle 状态
- 按下 `Alt+``：托盘图标变为 Recording 状态，开始录音
- 再按下 `Alt+``：停止录音，托盘图标变为 Transcribing 状态（短暂）
- ASR 识别完成后，文字自动出现在当前光标位置
- 状态回到 Idle，图标恢复
- 所有错误都有日志记录，不崩溃

**验证步骤:**
```bash
cargo build --release
./target/release/asr-boost
# 打开记事本/终端
# 按 Alt+` → 录音（说"你好"）
# 再按 Alt+` → 等待 ≤2秒 → "你好"出现在光标处
```

---

## Task 8: 云 ASR 引擎

**涉及文件 (4):**
- `src/asr/aliyun_asr.rs`
- `src/asr/openai_asr.rs`
- 更新 `src/asr/mod.rs`（注册新引擎）
- 更新配置结构

**验收标准:**
- 阿里云 ASR：使用 REST API，配置 appkey+token 后可识别
- OpenAI Whisper API：使用 REST API，配置 api_key 后可识别
- 托盘菜单可切换引擎
- 引擎失败时按 fallback 列表自动切换

---

## Task 9: 极简设置界面

**涉及文件 (3):**
- `src/settings/mod.rs`（winit 窗口）
- 更新 `src/tray/menu.rs`（"设置"菜单项）
- 更新 `src/main.rs`

**验收标准:**
- 点击"设置"弹出极小窗口（~400x300）
- 窗口内：热键绑定编辑、ASR 引擎选择、模型路径、API Key 输入
- 保存后自动重载配置
- 窗口关闭后不阻塞主循环

---

## Task 10: 打包 + 自启动

**涉及文件 (2):**
- `scripts/build-whisper.sh`
- `scripts/install-service.sh` + 各平台安装脚本

**验收标准:**
- 编译 whisper.cpp 脚本工作
- 各平台安装脚本将 asr-boost 设为开机自启
- 打包为单文件分发（Win: .msi / Mac: .dmg / Linux: .AppImage）
