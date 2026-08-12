pub mod monitor;
pub mod windowed;

crate::config_section! {
    pub(crate) struct WindowConfig => WINDOW_CONFIG_SECTION {
        section: "Window",
        order: 20,
        default_on: true,
        always_enabled: false,
        hidden: false,
        group: "display",
        comment: "显示设置",
        fields: {
            pub windowed: bool = false,
            comment: "窗口运行";
            pub framed: bool = true,
            comment: "显示窗口边框";
            pub monitor: i32 = 0,
            min: 0,
            comment: "显示器编号";
        }
    }
}
