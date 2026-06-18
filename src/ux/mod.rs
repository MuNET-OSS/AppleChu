pub mod dpi_aware;

use crate::config::Config;
use crate::util::api::Api;

pub fn init_all(api: &Api, config: &Config) {
    dpi_aware::init(api, config);
}
