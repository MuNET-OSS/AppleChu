# AppleChu

[English](README.en.md) | 简体中文

基于 [ChuModLoader](https://github.com/MuNET-OSS/ChuModLoader) 的 CHUNITHM Mod。

## 安装

1. 安装 ChuModLoader（将 `version.dll` 放到 `chusanApp.exe` 旁边）
2. 将 `AppleChu.dll` 复制到 `mods/` 目录
3. 启动游戏，`AppleChu.toml` 会自动生成
4. 编辑 `AppleChu.toml` 配置功能

## 功能

- 跳过启动画面
- 免费游玩
- 禁用选歌计时器
- 跳过地图动画
- 解锁游玩曲数上限（自定义最大曲数）
- 自定义各场景计时器
- 所有计时器 999
- 解锁 120fps
- 绕过 1080P/120Hz/AppUser 检测
- 强制共享音频 / 双声道输出
- 关闭网络加密 / TLS
- 自定义版本号文本
- 自定义 FREE PLAY 文本
- 自动游玩（智能屏蔽成绩）
- 退出确认对话框
- DPI 感知
- 切换窗口闪退修复 (D3D9)

## 配置

编辑游戏目录下的 `AppleChu.toml`，取消注释 `[Section]` 即可启用功能。

使用 [ChuChartManager](https://github.com/MuNET-OSS/ChuChartManager) 可图形化编辑配置。

## 构建

需要 Rust nightly + i686-pc-windows-msvc：

```bash
cargo build --release
```

输出: `target/i686-pc-windows-msvc/release/AppleChu.dll`

## 许可证

Apache-2.0
