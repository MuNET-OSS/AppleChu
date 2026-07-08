use std::fs;
use std::path::Path;

pub struct Config {
    pub base_dir: String,
    pub sections: toml::Table,
}

const DEFAULT_CONFIG: &str = r##"## 这是 AppleChu 的 TOML 配置文件
##
## - 井号 # 开头的行为注释，被注释掉的内容不会生效
##     - 为方便使用 VSCode 等编辑器进行编辑，被注释掉的配置内容使用一个井号 #，而注释文本使用两个井号 ##
## - 以方括号包裹的行，如 [System]，为一个栏目
##     - 将默认被注释（即默认禁用）的栏目取消注释即可启用
##     - 若要禁用一个默认启用的栏目，请在栏目下添加「key = value」配置项，删除它/注释它不会有效
## - 形如「键 = 值」为一个配置项
##     - 配置项应用到其上方最近的栏目，请不要在一个栏目被注释掉的情况下开启其配置项（会加到上一个栏目，无效）
##     - 当对应栏目启用时，配置项生效，无论是否将其取消注释
##     - 注释掉的配置项保留其注释中的默认值，默认值可能会随版本更新而变化
## - 该文件的格式和文字注释是固定的，配置文件将在启动时被重写，无法解析的内容将被删除
##
## 试试使用 ChuChartManager 图形化配置 AppleChu 吧！
## https://github.com/MuNET-OSS/ChuChartManager

Version = "1"

## =============================================================================
## 系统设置
## =============================================================================

[System]
## 店内联机基准机/从机
## false = 基准机（单机，或局域网中的标准机，默认）
## true  = 从机（局域网中有 2-4 台主机时，非标准机的那些设为 true）
LanSlave = false

## 机台模式
## SP  = 使用 SP 模式默认端口（LED: COM20/21, Aime: COM3, 启用 VFD）
## CVT = 使用 CVT 模式默认端口（LED: COM2/3, Aime: COM2, 禁用 VFD）
Mode = "SP"

## 显示器刷新率
## 1 = 60Hz（默认）
## 2 = 120Hz（仅当显示器刷新率 ≥120Hz 时选择，否则可能黑屏）
RefreshRate = 1

## =============================================================================
## 显示设置
## =============================================================================

[Window]
## 1 = 窗口运行；0 = 全屏运行
windowed = 1
## 1 = 有边框窗口；0 = 无边框窗口
framed = 1

## =============================================================================
## 外部 IO（不填则使用内置键盘/虚拟卡仿真）
## =============================================================================

## 控制器 IO DLL（TASOLLER / Yubideck 等第三方控制器）
[ChuniIo]
#path = "*.dll"
#path32 = "*.dll"
#path64 = "*.dll"

## 读卡器 IO DLL（第三方读卡器）
[AimeIo]
#path = "*.dll"
#path32 = "*.dll"
#path64 = "*.dll"

## =============================================================================
## 补丁功能
## =============================================================================

## 跳过启动画面
[SkipStartup]

## 免费游玩
[FreePlay]
#custom_text = ""

## 禁用选歌计时器
[DisableTimer]

## 跳过地图动画
[SkipMapAnimation]

## 解锁 120fps
[Unlock120fps]
#force = true

## 关闭网络加密（私服需要）
[DisableEncryption]

## 关闭 TLS（私服需要）
[DisableTLS]

## DPI 感知
[DpiAware]

## 切换窗口闪退修复
[DeviceLostFix]

## D3D9Ex 透明升级与快速重启
[D3D9Ex]
enable = true
device_lost_recover = true
fast_restart = true

## 网络请求日志（诊断用：输出游戏的 WinHTTP 请求到日志）
#[NetLog]

## 全国对战（将对战 UDP 转为 TCP 经服务端中继，reflector 地址由服务端下发）
#[NationalMatch]

## 自动游玩
#[Autoplay]
#hotkey = "Home"

## FPS 显示
#[FpsDisplay]

## 帧率锁定
#[FrameLock]
#fps = 60

## 解锁曲数上限
#[UnlockTracks]
#max = 3

## 绕过 1080P 检测
#[Bypass1080p]

## 绕过 120Hz 检测
#[Bypass120hz]

## 绕过 AppUser 检测
#[BypassAppUser]

## 强制共享音频（采样率必须 48000Hz）
#[ForceSharedAudio]

## 强制双声道
#[Force2chAudio]

## 自定义版本号
#[General]
#version_text = ""

## =============================================================================
## 高级配置（正常情况下无需修改，以下设备仿真默认启用）
## =============================================================================

## IO4 USB HID（JVS 按键/投币/红外）
#[Io4]
#enable = true

## 键盘输入（兼容 segatools 的 [io3] / [ir] / [slider] 键名）
#[Buttons]
#test = 0x70
#service = 0x71
#coin = 0x72
#ir = 0x20

#[Air]
#air1 = 0x34
#air2 = 0x35
#air3 = 0x36
#air4 = 0x37
#air5 = 0x38
#air6 = 0x39

#[Slider]
#cell1 = 0x4C
#cell2 = 0x4C
#cell3 = 0x4C
#cell4 = 0x4C

## 触摸条仿真（使用真实 AC 触摸条时设为 false，端口 COM1）
#[SliderDevice]
#enable = true

## Aime 读卡器（使用真实读卡器时设为 false）
#[Aime]
#enable = true
#aime_path = "aime.txt"
#felica_path = "felica.txt"
#authdata_path = "DEVICE\\authdata.bin"
#aime_gen = true
#felica_gen = false
#scan = 0x0D
#gen = 3
#proxy_flag = 2

## LED15093 灯板
#[Led15093]
#enable = true

## VFD 显示板（仅 SP 模式）
#[Vfd]
#enable = true
"##;

impl Config {
    pub fn load(base_dir: &str) -> Self {
        let path = Path::new(base_dir).join("AppleChu.toml");

        if !path.exists() {
            let _ = fs::write(&path, DEFAULT_CONFIG);
        }

        let content = fs::read_to_string(&path).unwrap_or_else(|_| DEFAULT_CONFIG.to_owned());
        let parsed = content
            .parse::<toml::Table>()
            .unwrap_or_else(|_| DEFAULT_CONFIG.parse().unwrap_or_default());

        let mut config = Self {
            base_dir: base_dir.to_owned(),
            sections: parsed,
        };
        config.derive_dipsw();
        config
    }

    fn derive_dipsw(&mut self) {
        let lan_slave = self.get_bool("System", "LanSlave", false);
        let refresh_rate_raw = self.get_int("System", "RefreshRate", 1);
        let refresh_120hz = refresh_rate_raw == 2;

        let dipsw1 = if lan_slave { 0 } else { 1 };
        let (dipsw2, dipsw3) = if refresh_120hz { (0, 0) } else { (1, 1) };

        let table = match self.sections.entry("System") {
            toml::map::Entry::Occupied(entry) => entry.into_mut(),
            toml::map::Entry::Vacant(entry) => entry.insert(toml::Value::Table(toml::Table::new())),
        };
        if let Some(table) = table.as_table_mut() {
            table.insert("dipsw1".into(), toml::Value::Integer(dipsw1));
            table.insert("dipsw2".into(), toml::Value::Integer(dipsw2));
            table.insert("dipsw3".into(), toml::Value::Integer(dipsw3));
        }
    }

    pub fn is_sp_mode(&self) -> bool {
        self.get_string("System", "Mode", "")
            .eq_ignore_ascii_case("sp")
    }

    pub fn is_enabled(&self, section: &str) -> bool {
        self.section(section).is_some_and(|table| {
            !table
                .get("Disabled")
                .and_then(toml::Value::as_bool)
                .unwrap_or(false)
        })
    }

    pub fn get_int(&self, section: &str, key: &str, default: i64) -> i64 {
        self.get_value(section, key)
            .and_then(toml::Value::as_integer)
            .unwrap_or(default)
    }

    pub fn get_int_alias(&self, keys: &[(&str, &str)], default: i64) -> i64 {
        keys.iter()
            .find_map(|(section, key)| self.get_value(section, key))
            .and_then(toml::Value::as_integer)
            .unwrap_or(default)
    }

    pub fn get_bool(&self, section: &str, key: &str, default: bool) -> bool {
        self.get_value(section, key)
            .and_then(|value| match value {
                toml::Value::Boolean(value) => Some(*value),
                toml::Value::Integer(value) => Some(*value != 0),
                toml::Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" | "on" => Some(true),
                    "0" | "false" | "no" | "off" => Some(false),
                    _ => None,
                },
                _ => None,
            })
            .unwrap_or(default)
    }

    pub fn get_bool_alias(&self, keys: &[(&str, &str)], default: bool) -> bool {
        keys.iter()
            .find_map(|(section, key)| self.get_value(section, key))
            .and_then(|value| match value {
                toml::Value::Boolean(value) => Some(*value),
                toml::Value::Integer(value) => Some(*value != 0),
                toml::Value::String(value) => match value.trim().to_ascii_lowercase().as_str() {
                    "1" | "true" | "yes" | "on" => Some(true),
                    "0" | "false" | "no" | "off" => Some(false),
                    _ => None,
                },
                _ => None,
            })
            .unwrap_or(default)
    }

    pub fn get_string(&self, section: &str, key: &str, default: &str) -> String {
        self.get_value(section, key)
            .and_then(toml::Value::as_str)
            .unwrap_or(default)
            .to_owned()
    }

    pub fn get_string_alias(&self, keys: &[(&str, &str)], default: &str) -> String {
        keys.iter()
            .find_map(|(section, key)| self.get_value(section, key))
            .and_then(toml::Value::as_str)
            .unwrap_or(default)
            .to_owned()
    }

    pub fn get_value(&self, section: &str, key: &str) -> Option<&toml::Value> {
        self.section(section).and_then(|table| {
            table.get(key).or_else(|| {
                table
                    .iter()
                    .find(|(entry_key, _)| entry_key.eq_ignore_ascii_case(key))
                    .map(|(_, value)| value)
            })
        })
    }

    fn section(&self, section: &str) -> Option<&toml::Table> {
        self.sections
            .get(section)
            .or_else(|| {
                self.sections
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(section))
                    .map(|(_, value)| value)
            })
            .and_then(toml::Value::as_table)
    }
}
