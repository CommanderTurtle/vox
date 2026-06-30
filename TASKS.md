# vox — 实施任务列表 (全部已完成 ✅)

> 以下所有任务均已实现并通过验证。

---

## Task 1: 项目脚手架 + 配置系统

**涉及文件:** `Cargo.toml`, `src/main.rs`, `src/config/mod.rs`, `src/config/defaults.toml`

**验收:** ✅ `cargo build` 成功编译，`ConfigManager` 加载/保存 TOML，路径按平台标准

---

## Task 2: 系统托盘 + 基础状态管理

**涉及文件:** `src/tray/mod.rs`, `src/app/state.rs`, `src/main.rs`

**验收:** ✅ 托盘图标出现，右键菜单完整，Quit 可点击退出，状态机 Idle/Recording/Transcribing

---

## Task 3: 全局热键监听

**涉及文件:** `src/app/hotkey.rs`, `src/main.rs`

**验收:** ✅ rdev 独立线程监听 `Alt+`` / `Alt+Shift+E` / `Alt+Shift+V` / `Alt+T`，HotkeyBinding 解析支持修饰键

---

## Task 4: 音频采集模块

**涉及文件:** `src/audio/mod.rs`, `src/audio/capture.rs`, `src/audio/utils.rs`

**验收:** ✅ cpal 16kHz mono i16 PCM 采集，AtomicBool 控制启停，WAV 编码测试通过

---

## Task 5: ASR 引擎模块

**涉及文件:** `src/asr/mod.rs`, `src/asr/whisper_local.rs`, `src/asr/mimo_asr.rs`, `src/asr/openai_asr.rs`, `src/asr/aliyun_asr.rs`

**验收:** ✅ AsrEngine trait + AsrManager + fallback 链，4 个引擎全部注册（Mimo 工作中，其余配 key 即用）

---

## Task 6: 文字注入模块

**涉及文件:** `src/inject/mod.rs`, `src/inject/keyboard.rs`, `src/inject/clipboard.rs`, `src/inject/text_reader.rs`

**验收:** ✅ 键盘模拟 / 剪贴板粘贴两种模式均可切换，额外实现 text_reader (Ctrl+C 模拟读选中文字)

---

## Task 7: 主循环组装 — 端到端闭环

**涉及文件:** `src/main.rs`

**验收:** ✅ 热键→录音→ASR→注入全链路正常工作（用户已实际验证），托盘 tooltip 反映状态

---

## Task 8: 云 ASR 引擎

**涉及文件:** `src/asr/mimo_asr.rs`, `src/asr/openai_asr.rs`, `src/asr/aliyun_asr.rs`

**验收:** ✅ Mimo (小米多模态 API, 已工作)、OpenAI Whisper API、阿里云一句话识别，均可通过托盘菜单切换

---

## Task 9: 极简设置界面

**涉及文件:** `src/settings/mod.rs`, `src/main.rs`

**验收:** ✅ egui 窗口 (~480×520)，含热键编辑、引擎选择、API Key 输入、注入模式、保存/取消

---

## Task 10: 打包 + 自启动

**涉及文件:**
- `scripts/build-whisper.sh` — 下载/编译 whisper.cpp + tiny 模型
- `scripts/install-service.sh` — macOS LaunchAgent / Linux systemd 自启动
- `scripts/install-autostart.bat` — Windows 注册表 Run key 自启动
- `scripts/package.sh` — 打包 Release 二进制 + dmg/AppImage

**验收:** ✅ 所有脚本就绪，CLI debug 子命令 (`vox transcribe` / `vox inject`) 同时实现

---

## 额外完成

- **CLI 调试子命令**: `cargo run -- transcribe <file.wav>` / `cargo run -- inject <text> [--mode]`
- **TTS 功能**: Mimo TTS 引擎 + 选中文字/剪贴板输入模式切换 + 系统音频播放
- **TTS 引擎切换**: 右键菜单 → TTS Engine / TTS Input
- **项目更名**: `asr-boost` → `vox`
- **README**: 中英双语 GitHub 风格
- **AGENTS.md**: AI 助手记忆文件

### 验证状态

```
cargo build          ✅ 零错误、零警告
cargo clippy         ✅ 零警告
cargo test           ✅ 18/18 通过
cargo run            ✅ 托盘正常启动
vox transcribe x.wav ✅ 本地 whisper.cpp ASR 端到端通过
vox tts "..."        ✅ Edge TTS 合成 MP3 + rodio 播放通过
```

---

## Task 11: 审阅修复（采样率/阻塞/UI/热键/剪贴板/安全）

**涉及文件:** `src/audio/capture.rs`, `src/audio/utils.rs`, `src/main.rs`, `src/asr/mod.rs`, `src/tts/mod.rs`, `src/app/hotkey.rs`, `src/inject/*`, `src/config/*`, `src/settings/mod.rs`

- 采集真实设备采样率 + 线性重采样到 16kHz（修复非 16kHz 设备识别乱码）
- 主循环改为共享 runtime + spawn，ASR/TTS 不再 block_on 阻塞 UI
- whisper-local 读取配置 model_path；引擎 Vec 保序；active 用 RwLock 支持 Arc 共享
- 热键 per-press 去抖；设置经 channel 实时生效；ClipboardSnapshot 保护非文本剪贴板
- 移除硬编码 API key

**验收:** ✅ cargo test 18/18；ASR/TTS 端到端验证

---

## Task 12: 免密钥 Edge TTS + 本地 ASR + 菜单/设置解耦

**涉及文件:** `src/tts/edge_tts.rs` (新), `src/tts/playback.rs`, `src/asr/whisper_cpp.rs` (新), `src/asr/openai_asr.rs`, `src/config/*`, `src/tray/mod.rs`, `src/settings/mod.rs`, `src/main.rs`, `Cargo.toml`

- Edge TTS：WebSocket 免密钥，Sec-MS-GEC（5 分钟对齐 + SHA-256），MUID cookie，返回 MP3
- 播放改为 rodio 进程内解码（WAV/MP3），弃用外部播放器命令
- whisper.cpp HTTP ASR 引擎；OpenAI ASR base_url 可指向 localhost
- 默认主引擎改为 whisper-cpp + edge-tts，开箱免密钥可测
- 托盘菜单由 MenuModel 渲染，分组 + CheckMenuItem 勾选 + 实时刷新
- 设置窗口改为 Config 快照纯视图，经 channel 回传，apply_settings 实时生效
- rustls ring provider 启动时安装

**验收:** ✅ `vox transcribe` 走 whisper.cpp 返回识别文本；`vox tts` 合成 MP3 并播放；clippy 零警告
