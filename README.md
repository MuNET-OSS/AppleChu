# AppleChu

集成 Mod 加载器的 CHUNITHM 启动代理与功能补丁

<p align="center">
  <a href="#许可证"><img src="https://img.shields.io/badge/license-Apache--2.0-blue.svg" alt="License"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20x86-lightgrey.svg" alt="Platform">
  <img src="https://img.shields.io/badge/rust-nightly-orange.svg" alt="Rust">
</p>

## 目录

- [AppleChu](#applechu)
  - [目录](#目录)
  - [安装](#安装)
  - [配置](#配置)
  - [使用](#使用)
  - [功能](#功能)
  - [IO 仿真](#io-仿真)
  - [构建](#构建)
  - [许可证](#许可证)

## 安装

1. 将 `winhttp.dll` 放到 `chusanApp.exe` 旁边，将 `winmm.dll` 放到 `amdaemon.exe` 旁边 通常两者都在 `bin/` 目录下
2. 启动 `chusanApp.exe` 和 `AM Daemon.exe` ，`AppleChu.toml` 会在游戏目录自动生成
3. 按需编辑 `AppleChu.toml` 启用功能

其他兼容 ChuMod API 的 DLL 仍可放入 `mods/` 目录自动加载。（包括原本依靠注入进 `chusanApp.exe` 的 DLL 也可以自动加载）

> [!TIP]
> 预编译的 `winhttp.dll` 和 `winmm.dll` 可在 [Releases](https://github.com/MuNET-OSS/AppleChu/releases) 页面下载

## 配置

所有功能通过游戏目录下的 `AppleChu.toml` 控制。默认关闭的栏目取消注释即可启用；默认开启的栏目可用 `enable = false` 关闭。程序会根据各模块声明的 schema 校验并规范化配置文件。

> [!NOTE]
> 如需图形化编辑，可使用 [ChuChartManager](https://github.com/MuNET-OSS/ChuChartManager)

## 使用

直接运行 `AM Daemon.exe` 和 `chusanApp.exe` 即可，AM Daemon的 Json 配置可以在 AppleChu.toml 里自动开启补全

## 功能

| 分类 | 功能 | 说明 |
| --- | --- | --- |
| 常用 | 跳过启动画面 | 跳过开机动画直接进入游戏 |
| 常用 | 免费游玩 | 强制 FREE PLAY，可自定义显示文本 |
| 常用 | 禁用选歌计时器 | 选歌界面不再倒计时 |
| 常用 | 跳过地图动画 | 跳过地图过场动画 |
| 游戏 | 解锁曲数上限 | 自定义单局最大游玩曲数 |
| 游戏 | 自定义计时器 | 单独调整各场景计时器 |
| 游戏 | 全部计时器 999 | 将所有计时器统一拉满 |
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
| 兼容 | 修复自定义 ReleaseTag 映射 | 修复新版本 Achievement 系统对自定义 ReleaseTag ID闪退问题（内置） |
| 体验 | 退出确认 | 关闭游戏前弹出确认框 |
| 体验 | 设备丢失修复 | 修复切换窗口导致的 D3D9 闪退 |
| 通用 | 自定义版本号 | 覆盖屏幕上显示的版本文本 |

## IO 仿真

AppleChu 内置一套与 segatools 配置和 API 完全兼容的游戏侧 IO 仿真，无需 segatools 即可驱动游戏输入与外设

| 模块 | 配置段 | 说明 |
| --- | --- | --- |
| 控制器 IO DLL | `[ChuniIo]` | 加载外部 `chuniio` DLL，支持 `path` / `path32` / `path64` |
| 读卡器 IO DLL | `[AimeIo]` | 加载外部 `aimeio` DLL |
| 按键输入 | `[Buttons]` | Test / Service / 投币 / AIR 模拟键位 |
| AIR 输入 | `[Air]` | AIR 1-6 键位映射 |
| 触摸条键位 | `[Slider]` | 键盘模拟触摸条（Cell 1-32） |
| IO4 仿真 | `[Io4]` | 内置 IO4 主控仿真 |
| 触摸条仿真 | `[SliderDevice]` | 内置触摸条设备仿真 |
| Aime 读卡器 | `[Aime]` | 从 `aime.txt` / `felica.txt` 读卡，支持扫卡键 |
| LED15093 灯板 | `[Led15093]` | LED15093 灯板仿真 |
| VFD 显示板 | `[Vfd]` | VFD 显示板仿真 |

> [!IMPORTANT]
> 键位值为 Windows 虚拟键码（VK code）。若同时配置了外部 `[ChuniIo]` / `[AimeIo]` DLL，则优先使用外部 DLL

## 构建

需要 Rust nightly 工具链与 `i686-pc-windows-msvc` 和 `x86_64-pc-windows-msvc` 目标：

```bash
rustup target add i686-pc-windows-msvc
rustup target add x86_64-pc-windows-msvc
cargo build --release
cargo build --release --target x86_64-pc-windows-msvc -p applechu-amdaemon
```

产物位于：

```text
target/i686-pc-windows-msvc/release/winhttp.dll
target/x86_64-pc-windows-msvc/release/winmm.dll
```

## 许可证

[Apache-2.0](LICENSE)
