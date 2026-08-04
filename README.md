# AppleChu

集成 Mod 加载器的 CHUNITHM 启动代理与功能补丁

<p align="center">
  <a href="#许可证"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20x86-lightgrey.svg" alt="Platform">
  <img src="https://img.shields.io/badge/rust-nightly-orange.svg" alt="Rust">
</p>

## 安装

1. 将 `winhttp.dll` 放到 `chusanApp.exe` 旁边
2. 启动游戏，`AppleChu.toml` 会在游戏目录自动生成
3. 按需编辑 `AppleChu.toml` 启用功能

其他兼容 ChuMod API 的 DLL 仍可放入 `mods/` 目录自动加载。

> [!TIP]
> 预编译的 `winhttp.dll` 可在 [Releases](https://github.com/MuNET-OSS/AppleChu/releases) 页面下载

## 配置

所有功能通过游戏目录下的 `AppleChu.toml` 控制。栏目存在时启用，注释栏目时关闭。程序使用内置统一 schema 校验并规范化配置文件。

> [!NOTE]
> 如需图形化编辑，可使用 [ChuChartManager](https://github.com/MuNET-OSS/ChuChartManager)

## 功能

| 分类 | 功能 | 说明 |
| --- | --- | --- |
| 常用 | 跳过启动画面 | 跳过开机动画直接进入游戏 |
| 常用 | 免费游玩 | 强制 FREE PLAY，可自定义显示文本 |
| 常用 | 禁用选歌计时器 | 选歌界面不再倒计时 |
| 常用 | 跳过地图动画 | 跳过地图过场动画 |
| 游戏 | 解锁曲数上限 | 自定义单局最大游玩曲数 |
| 游戏 | 自定义计时器 | 单独调整各场景计时器 |
| 游戏 | 自动游玩 | Autoplay，默认 `Home` 键切换，可配置键位 |
| 显示 | 解锁 120fps | 解除帧率限制 |
| 显示 | FPS 显示 | 屏幕内显示实时帧率（需 d3d9 代理） |
| 显示 | 帧率锁定 | 锁定到自定义目标帧率（需 d3d9 代理） |
| 显示 | 绕过 1080P / 120Hz | 跳过分辨率与刷新率检测 |
| 显示 | DPI 感知 | 启用高 DPI 适配 |
| 音频 | 强制共享音频 | 使用共享模式输出 |
| 音频 | 强制双声道 | 强制 2ch 输出 |
| 网络 | 关闭加密 | 禁用网络加密 |
| 网络 | 关闭 TLS | 禁用 TLS |
| 兼容 | 绕过 AppUser | 跳过 AppUser 检测 |
| 通用 | 自定义版本号 | 覆盖屏幕上显示的版本文本 |

## IO 仿真

AppleChu 内置完整的游戏侧 IO 仿真，可直接驱动游戏输入与外设

| 模块 | 配置段 | 说明 |
| --- | --- | --- |
| 控制器 IO DLL | `[ChuniIo]` | 加载外部 `chuniio` DLL，支持 `path` / `path32` / `path64` |
| 读卡器 IO DLL | `[AimeIo]` | 加载外部 `aimeio` DLL |
| 按键输入 | `[Buttons]` | Test / Service / 投币 / AIR 模拟键位 |
| AIR 输入 | `[Air]` | AIR 1-6 键位映射 |
| 触摸条键位 | `[Slider]` | 键盘模拟触摸条（Cell 1-32） |
| IO4 仿真 | `[Io4]` | 内置 IO4 主控仿真 |
| 触摸条仿真 | `[SliderDevice]` | 内置触摸条设备仿真 |
| Aime 读卡器 | `[Aime]` | 从 `DEVICE\aime.txt` / `DEVICE\felica.txt` 读卡，支持扫卡键 |
| LED15093 灯板 | `[Led15093]` | LED15093 灯板仿真 |
| VFD 显示板 | `[Vfd]` | VFD 显示板仿真 |

> [!IMPORTANT]
> 键位值为 Windows 虚拟键码（VK code）。若同时配置了外部 `[ChuniIo]` / `[AimeIo]` DLL，则优先使用外部 DLL

AM Daemon 专用的 `[Dns]`、`[Keychip]`、`[Epay]`、`[Ewf]`、`[NetEnv]`、`[OpenSsl]` 等栏目也使用同一份
`AppleChu.toml`，但只由 64 位 `winmm.dll` 读取和执行；游戏侧不会复制这些实现。

配置栏目本身是功能总开关，栏目存在即启用，注释后即关闭。
CCM 使用最终 `winhttp.dll` 的 `.acmani` PE section 提取 schema，schema 不作为外部运行时文件分发。

## 构建

游戏侧 `winhttp.dll` 需要 Rust nightly 工具链与 `i686-pc-windows-msvc` 目标：

```bash
rustup target add i686-pc-windows-msvc
cargo build --release
```

产物位于：

```text
target/i686-pc-windows-msvc/release/winhttp.dll
```

AM Daemon 侧的 `winmm.dll` 是独立的 64 位代理，必须单独使用 x64 MSVC
目标编译。不要把它放进默认 i686 构建目录：

```powershell
rustup target add x86_64-pc-windows-msvc
cargo build --release -p applechu-amdaemon --target x86_64-pc-windows-msvc
```

生成文件位于：

```text
target/x86_64-pc-windows-msvc/release/winmm.dll
```

## 许可证

[Apache-2.0](LICENSE)
