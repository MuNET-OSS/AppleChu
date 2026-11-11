# QA 记录

## Rust 构建目标

- 不要使用默认目标直接运行 `cargo test --workspace`：`applechu` 游戏 DLL 使用 `i686-pc-windows-msvc`，`applechu-amdaemon` 只支持 `x86_64-pc-windows-msvc`。
- 发布构建以 `build.ps1` 为准；它会分别构建 i686 `winhttp.dll` 和 x64 `winmm.dll`。
- 跨架构改动至少运行 `cargo check -p applechu --target x86_64-pc-windows-msvc` 和 `cargo check -p applechu-amdaemon --target x86_64-pc-windows-msvc`，避免默认 i686 构建遗漏 x64 条件代码。

## 构建产物检查

- Cargo build script 的 `OUT_DIR` 目录层级会随目标与 profile 改变；定位 profile 目录时应查找固定的 `build` 目录边界，不能依赖 `ancestors().nth(...)`。
- Windows PowerShell 不保证提供 `ConvertFrom-Toml`；配置 TOML 校验应优先使用项目自身的 Rust 解析测试。
- `applechu-schema-export` 依次接收 Rust 源目录和输出目录，不能只传输出目录。

## Linux 交叉构建

- `xwin splat` 默认会移动 `unpack` 缓存，不能并行给多个架构复用同一个 cache；应加 `--copy`，并给每个架构使用独立的输出目录。
- MSVC 目标还需要显式设置 `CARGO_TARGET_<TRIPLE>_LINKER=lld-link`，只配 `RUSTFLAGS` 会报找不到 `link.exe`。
- build script 需要真实 PE 转发 DLL；构建 i686 时设置 `APPLECHU_WINHTTP_DEV_DLL`，构建 x86_64 时设置 `APPLECHU_WINMM_DEV_DLL`，可分别指向 Wine 对应架构的 DLL。
- `cargo xwin build` 的 splat（`~/.cache/cargo-xwin/xwin`）只展开 x86_64 库，i686 链接会报 `lld-link: error: could not open 'dbghelp.lib'`；i686 要用已有的 `~/.cache/applechu/winsdk-x86` 手工传 `-Lnative` 并设 `CARGO_TARGET_I686_PC_WINDOWS_MSVC_LINKER=lld-link`。
- Windows 测试可先 `cargo test --no-run`，再用项目内独立的 `WINEPREFIX` 直接执行 test exe；完整 `applechu` 套件会在 Wine 的 D3D9/内存补丁测试里 abort，配置相关定向测试可以正常执行。
