mod autoplay;

pub use autoplay::{is_enabled, was_used};

use crate::config::Config;
use crate::util::api::Api;

pub fn init_all(api: &Api, config: &Config) {
    if config.is_enabled("Autoplay") {
        autoplay::init(api, config);
        autoplay::init_upload_guard(api);
    }
}

pub fn shutdown_all() {
    autoplay::shutdown_upload_guard();
    autoplay::shutdown();
}
