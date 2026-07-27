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
vox tts "..."        ✅ 豆包 TTS (seed-tts-2.0) 合成 PCM->WAV + 播放通过
vox transcribe ...   ✅ 豆包 ASR (Plan 路径 + Ark key) 端到端识别通过
bash scripts/package.sh ✅ 打包 dist/vox-v0.1.2-windows-x86_64.zip 通过
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

---

## Task 13: Push-to-Talk 录音模式 + 菜单切换 + 多平台打包

**涉及文件:** `src/app/state.rs`, `src/config/mod.rs`, `src/config/defaults.toml`, `src/main.rs`, `src/tray/mod.rs`, `src/settings/mod.rs`, `.github/workflows/release.yml` (新), `scripts/package.sh`, 文档

- `RecordMode` 枚举 (PushToTalk/Toggle) + `from_str`/`as_str`/`display_name`
- config `general.record_mode` (默认 `ptt`，`#[serde(default)]` 兼容)
- `toggle_recording` 拆分为 `start_recording`/`stop_recording`；PTT 模式按下开始、松手停止
- 托盘菜单加 `Record Mode` 子菜单 (CheckMenuItem)；设置窗口加下拉；实时生效
- `package.sh` 重写（跨平台 zip/tar.gz）；`.github/workflows/release.yml` 三平台构建 + Release

**验收:** ✅ `cargo build --release` + `cargo test` 18/18 + `cargo clippy` 零警告通过；`bash scripts/package.sh` 本地打包验证通过，产物 `dist/vox-v0.1.2-windows-x86_64.zip` 含 `vox.exe` + `README.md` + `LICENSE` + `config-example/config.toml`。跨平台 GitHub Actions release workflow 就绪，待推送 tag 触发三平台构建。

---

# 审计发现的工作项（按优先级）

> 以下任务来自代码/CI/跨平台审计，逐条执行。

## Task 14: [高] Linux 全局热键 uinput 权限文档缺失

**涉及文件:** `README.md`, `AGENTS.md`, `src/app/hotkey.rs`

- rdev 在 Linux 监听全局热键需 `/dev/uinput` 写权限；用户须加入 `input` 组或 `chmod 0660 /dev/uinput`
- 当前文档零提及，Linux 用户首次运行热键会静默失效（仅 log error 后退出）
- README 加 Linux 前置条件说明；热键监听失败时给更明确提示

**验收:** README/AGENTS 含 Linux uinput 权限说明；hotkey 启动失败日志含可操作的修复指引

---

## Task 15: [高] 豆包 ASR 端到端验证（需专用 key）

**涉及文件:** `src/asr/doubao_asr.rs`（修复错误帧解析 + 关闭 gzip 压缩）

- 代码与协议链路已验证；排查发现 Ark key 走 Plan 路径（`/api/v3/plan/sauc/`）可同时用于 TTS + ASR
- 修复二进制帧解析 bug：错误帧（msg_type=0xF）布局为 `[4B header][4B error_code][4B msg_size][msg]`，与正常帧不同，原先误把 error_code 当 payload_size 读取
- 关闭 payload gzip 压缩：服务端对客户端 gzip payload 报 `unable to ungzip payload: EOF`，改为 compression=NONE 后正常
- `parse_error_frame` 单独处理错误帧，提取 JSON `{"error":"..."}` 内层消息

**验收:** ✅ `vox transcribe <录音.wav>` 走 doubao-asr（Plan 路径 + Ark key）返回识别文本"测试"

---

## Task 16: [中] 补日常 CI workflow（push/PR 触发 test+clippy+fmt）

**涉及文件:** `.github/workflows/ci.yml` (新)

- 现状仅 `release.yml`（tag 触发），push 到 main / PR 时不跑任何检查
- 源码 18 个单测从不被 CI 执行
- 新增 ci.yml：push/PR 触发 `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` + `cargo test`

**验收:** ci.yml 推送后在 GitHub Actions 看到三步全绿（本地已验证 fmt+clippy+test 通过；fmt 已全量应用 `cargo fmt`）

---

## Task 17: [中] 新引擎纯逻辑补单元测试

**涉及文件:** `src/asr/doubao_asr.rs`, `src/tts/doubao_tts.rs`, `src/app/state.rs`, `src/tts/edge_tts.rs`

- `doubao_asr`: `build_frame`/`parse_server_frame` 往返、`extract_result_text`、`gzip_bytes`↔`maybe_gunzip` 往返、错误帧/截断帧分支
- `doubao_tts`: `parse_frame` 五分支（audio/skip/end/错误码/非法 JSON）
- `state`: `RecordMode::from_str`/`as_str` round-trip + 大小写容错
- `edge_tts`: `lang_for_voice`、`build_ssml`（XML 转义）

**验收:** ✅ `cargo test` 18→39 通过（+21）；clippy 零警告；fmt 干净。覆盖错误帧解析回归、gzip 往返、NDJSON 五分支、RecordMode 转换、SSML 转义

---

## Task 18: [中] 迁移已废弃的 actions-rs/cargo Action

**涉及文件:** `.github/workflows/release.yml`

- `actions-rs/cargo@v1` 已 archived，触发 Node.js 20 deprecation 警告
- 改用 `dtolnay/rust-toolchain` + 直接 `run: cargo build`

**验收:** ✅ release.yml 改用 `run: cargo build`(dtolnay 工具链已装)；移除 `actions-rs/cargo`；无 deprecation 警告

---

## Task 19: [低] macOS 分发包未签名/公证

**涉及文件:** `.github/workflows/release.yml`

- 裸二进制被 Gatekeeper 拦截，用户需手动 `xattr -d com.apple.quarantine`
- 需 Apple Developer 账号做 codesign + notarize（成本较高，待定）

**验收:** 待定（依赖 Apple Developer 账号）

---

## Task 20: [低] .gitignore 补全

**涉及文件:** `.gitignore`

- 补 `*.log`、`.vscode/`、`.idea/`、`.DS_Store`、`Thumbs.db`、`.env`、`*.local.toml`
- 有 API key 场景，防密钥泄露

**验收:** .gitignore 含上述规则

---

## Task 21: [低] tray-icon 初始化失败上抛错误

**涉及文件:** `src/tray/mod.rs`

- 当前 `create_default_icon`/`TrayIconBuilder::build()` 失败仅 log error 后 return，托盘静默消失但程序继续跑
- 改为上抛错误或至少在托盘缺失时给用户明确反馈

**验收:** ✅ 托盘初始化失败时 log::error 含完整上下文（失败原因 + 热键/CLI 仍可用 + Linux 桌面环境提示），不再静默 return

---

## Task 22: [低] 配置双写 + defaults.toml 静默吞错

**涉及文件:** `Cargo.toml`, `.github/workflows/release.yml`, `scripts/package.sh`

- `[profile.release]` 与 release.yml env 重复设 LTO/codegen-units，改一处易忘另一处
- package.sh 复制 defaults.toml 用 `|| true` 静默吞错
- 统一 profile 配置到 Cargo.toml，workflow 不重复设；移除 `|| true`

**验收:** ✅ release.yml 移除重复 LTO env（单一来源 Cargo.toml `[profile.release]`）；README/defaults.toml 复制移除 `|| true`（缺失即报错）；LICENSE 保留 `|| true`（非运行必需）
