crate::config_section! {
    pub struct NetEnvConfig => NETENV_CONFIG_SECTION {
        section: "NetEnv",
        order: 370,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "网络环境模拟",
        fields: {
            pub addr_suffix: u32 = 11,
            advanced: true;
            pub router_suffix: u32 = 254,
            advanced: true;
            pub mac_addr: String = String::from("01:02:03:04:05:06"),
            advanced: true;
            pub broadcast: String = String::from("255.255.255.255"),
            advanced: true;
        }
    }
}
