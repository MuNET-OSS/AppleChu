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

#[applechu_macros::config_section(stage = Platform, order = 70)]
pub fn init(api: &Api, _config: &DvdConfig) {
    if vfs::root_cstring("option").is_some() {
        api.log_info("DVD path hook uses VFS option mount");
    }
}
