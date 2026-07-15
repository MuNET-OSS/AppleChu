use crate::config::Config;
use crate::platform::vfs;
use crate::util::api::Api;

crate::config_section! {
    pub(crate) struct DvdConfig => DVD_CONFIG_SECTION {
        section: "DVD",
        order: 930,
        default_enabled: true,
        always_enabled: false,
        hidden: true,
        comment: "DVD 路径模拟",
        fields: {}
    }
}

pub fn init(api: &Api, config: &Config) {
    if !config
        .section::<DvdConfig>()
        .is_some_and(|config| config.enabled)
    {
        return;
    }

    if vfs::root_cstring("option").is_some() {
        api.log_info("DVD path hook uses VFS option mount");
    }
}
