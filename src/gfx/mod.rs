pub mod d3d9;
pub mod monitor;
pub mod windowed;

use crate::config::Config;
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct WindowConfig => WINDOW_CONFIG_SECTION {
        section: "Window",
        order: 20,
        default_enabled: true,
        always_enabled: false,
        hidden: false,
        comment: "显示设置",
        fields: {
            pub windowed: bool = true,
            comment: "窗口运行";
            pub framed: bool = true,
            comment: "显示窗口边框";
            pub monitor: i32 = 0,
            comment: "显示器编号";
        }
    }
}

pub fn init_all(api: &Api, config: &Config) {
    windowed::init(api, config);
    monitor::init(api, config);
    d3d9::init(api, config);
}
