crate::config_section! {
    pub struct OpenSslConfig => OPENSSL_CONFIG_SECTION {
        section: "OpenSsl",
        order: 975,
        default_on: true,
        always_enabled: false,
        hidden: true,
        comment: "AM Daemon OpenSSL 兼容",
        fields: {
            pub force_legacy_sha: bool = false,
            comment: "强制禁用 OpenSSL SHA 扩展路径";
        }
    }
}
