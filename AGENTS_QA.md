# QA 记录

## Rust 构建目标

- 不要使用默认目标直接运行 `cargo test --workspace`：`applechu` 游戏 DLL 使用 `i686-pc-windows-msvc`，`applechu-amdaemon` 只支持 `x86_64-pc-windows-msvc`。
- 发布构建以 `build.ps1` 为准；它会分别构建 i686 `winhttp.dll` 和 x64 `winmm.dll`。
- 跨架构改动至少运行 `cargo check -p applechu --target x86_64-pc-windows-msvc` 和 `cargo check -p applechu-amdaemon --target x86_64-pc-windows-msvc`，避免默认 i686 构建遗漏 x64 条件代码。

## 构建产物检查

- Cargo build script 的 `OUT_DIR` 目录层级会随目标与 profile 改变；定位 profile 目录时应查找固定的 `build` 目录边界，不能依赖 `ancestors().nth(...)`。
- Windows PowerShell 不保证提供 `ConvertFrom-Toml`；配置 TOML 校验应优先使用项目自身的 Rust 解析测试。
