pub mod dpi_aware;
pub mod exit_confirm;

use crate::config::Config;
use crate::util::api::Api;

pub fn init_all(api: &Api, config: &Config) {
    dpi_aware::init(api, config);
    exit_confirm::init(api, config);
}
