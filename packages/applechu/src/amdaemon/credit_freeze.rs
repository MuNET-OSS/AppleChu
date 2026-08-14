crate::config_section! {
    pub struct CreditFreezeConfig => CREDIT_FREEZE_CONFIG_SECTION {
        section: "CreditFreeze",
        order: 140,
        default_on: false,
        always_enabled: false,
        hidden: false,
        comment: "阻止 credit 被消耗",
        fields: {}
    }
}
