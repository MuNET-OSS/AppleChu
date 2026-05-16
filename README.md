# AppleChu

CHUNITHM mod powered by [ChuModLoader](https://github.com/MuNET-OSS/ChuModLoader).

## Installation

1. Install ChuModLoader (place `version.dll` next to `chusanApp.exe`)
2. Copy `AppleChu.dll` to `mods/`
3. Launch the game — `AppleChu.toml` will be generated automatically
4. Edit `AppleChu.toml` to configure features

## Features

- Skip startup screen
- Free play
- Disable song selection timer
- Skip map animation
- Unlock track limit (custom max tracks)
- Custom scene timers
- All timers 999
- Unlock 120fps
- Bypass 1080p/120Hz/AppUser checks
- Force shared audio / 2ch output
- Disable network encryption / TLS
- Custom version text
- Custom FREE PLAY text
- Autoplay with smart score blocking
- Exit confirmation dialog
- DPI awareness

## Configuration

Edit `AppleChu.toml` in the game directory. Uncomment a `[Section]` to enable it.

Use [ChuChartManager](https://github.com/MuNET-OSS/ChuChartManager) for graphical configuration.

## Build

Requires Rust nightly with `i686-pc-windows-msvc` target:

```bash
cargo build --release
```

Output: `target/i686-pc-windows-msvc/release/AppleChu.dll`

## License

Apache-2.0
