use crate::config::Config;
use crate::platform::vfs;
use crate::util::api::Api;

pub fn init(api: &Api, config: &Config) {
    if !config.get_bool("DVD", "enable", true) {
        return;
    }

    if vfs::root_cstring("option").is_some() {
        api.log_info("DVD path hook uses VFS option mount");
    }
}
