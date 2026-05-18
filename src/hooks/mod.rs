pub mod autoplay;
pub mod smart_upload;

use crate::config::Config;
use crate::util::api::Api;

pub fn init_all(api: &Api, config: &Config) {
    if config.is_enabled("Autoplay") {
        autoplay::init(api, config);
        smart_upload::init(api);
    }
}

pub fn shutdown_all() {
    smart_upload::shutdown();
    autoplay::shutdown();
}
