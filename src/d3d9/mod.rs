mod hook;
mod recovery;

use crate::config::Config;
use crate::util::api::Api;

pub fn init_all(api: &Api, config: &Config) {
    if config.is_enabled("DeviceLostFix") {
        hook::init(api);
    }
}
