crate::config_section! {
    pub struct KeychipConfig => KEYCHIP_CONFIG_SECTION {
        section: "Keychip",
        order: 40,
        default_on: true,
        always_enabled: false,
        hidden: false,
        comment: "KeyChip 模拟",
        fields: {
            pub keychip_id: String = String::from("A69E-01A88888888"),
            key: "id",
            emit_default: true,
            comment: "Keychip ID";
            pub game_id: String = String::from("SDHD");
            pub platform_id: String = String::new(),
            advanced: true,
            comment: "留空时使用当前平台默认值";
            pub region: u32 = 1,
            advanced: true;
            pub billing_type: u32 = 1,
            advanced: true;
            pub system_flag: u32 = 0x64,
            advanced: true;
            pub subnet: String = String::from("192.168.139.0"),
            advanced: true;
            pub billing_ca: String = String::from("DEVICE\\ca.crt"),
            advanced: true;
            pub billing_pub: String = String::from("DEVICE\\billing.pub"),
            advanced: true;
        }
    }
}
