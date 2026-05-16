use std::fs;
use std::path::Path;

pub struct Config {
    pub sections: toml::Table,
}

const DEFAULT_CONFIG: &str = r##"## AppleChu v1.0.0
##
## - 井号 # 开头的行为注释，被注释掉的内容不会生效
## - 以方括号包裹的行，如 [SkipStartup]，为一个功能
##     - 将默认被注释的功能取消注释即可启用
##     - 若要禁用一个默认启用的功能，请在其下添加 Disabled = true
## - 形如 key = value 为一个配置项
##     - 配置项应用到其上方最近的功能
##     - 当对应功能启用时，配置项生效
##     - 注释掉的配置项保留其默认值
## - 该文件会在启动时被重写，无法解析的内容将被删除
##
## 使用 ChuChartManager 可图形化编辑
## https://github.com/MuNET-OSS/ChuChartManager

Version = "1"

## 通用设置
[General]
## 自定义版本号文本（留空不修改）
#version_text = ""

## 跳过启动画面
[SkipStartup]

## 免费游玩
[FreePlay]
## 自定义显示文本（留空使用默认 FREE PLAY）
#custom_text = ""

## 禁用选歌计时器
[DisableTimer]

## 跳过地图动画
[SkipMapAnimation]

## 解锁游玩曲数上限
#[UnlockTracks]
#max = 3

## 自定义各场景计时器
#[CustomTimers]
#map_select = 60
#ticket_select = 60
#course_select = 60

## 设置所有计时器为 999
#[AllTimers999]

## 自动游玩
## 只屏蔽成绩数据，角色/设置/地图进度正常保存
#[Autoplay]
#block_playlog = true
#block_music_detail = true

## 解锁 120fps
[Unlock120fps]
#force = true

## 绕过 1080P 检测
#[Bypass1080p]

## 绕过 120Hz 检测
#[Bypass120hz]

## DPI 感知
[DpiAware]

## 强制共享音频，系统采样率必须为 48000Hz
## 此项可能会增加延迟
#[ForceSharedAudio]

## 强制双声道输出
#[Force2chAudio]

## 关闭网络加密
## 私服联网需要开启此项
[DisableEncryption]

## 关闭 TLS
[DisableTLS]

## 绕过 AppUser 检测
#[BypassAppUser]

## 退出确认对话框
[ExitConfirm]

## 切换窗口闪退修复
[DeviceLostFix]
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

        Self { sections: parsed }
    }

    pub fn is_enabled(&self, section: &str) -> bool {
        self.sections
            .get(section)
            .and_then(toml::Value::as_table)
            .is_some_and(|table| !table.get("Disabled").and_then(toml::Value::as_bool).unwrap_or(false))
    }

    pub fn get_int(&self, section: &str, key: &str, default: i64) -> i64 {
        self.section(section)
            .and_then(|table| table.get(key))
            .and_then(toml::Value::as_integer)
            .unwrap_or(default)
    }

    pub fn get_bool(&self, section: &str, key: &str, default: bool) -> bool {
        self.section(section)
            .and_then(|table| table.get(key))
            .and_then(toml::Value::as_bool)
            .unwrap_or(default)
    }

    pub fn get_string(&self, section: &str, key: &str, default: &str) -> String {
        self.section(section)
            .and_then(|table| table.get(key))
            .and_then(toml::Value::as_str)
            .unwrap_or(default)
            .to_owned()
    }

    fn section(&self, section: &str) -> Option<&toml::Table> {
        self.sections.get(section).and_then(toml::Value::as_table)
    }
}
