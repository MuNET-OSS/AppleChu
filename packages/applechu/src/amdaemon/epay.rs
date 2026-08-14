crate::config_section! {
    pub struct EpayConfig => EPAY_CONFIG_SECTION {
        section: "Epay",
        order: 972,
        default_on: true,
        always_enabled: false,
        hidden: true,
        comment: "AM Daemon ThincaPayment 兼容",
        fields: {
            pub hook: bool = true,
            comment: "使用本地支付接口桩，使 AM Daemon 可在无支付终端时启动";
        }
    }
}
