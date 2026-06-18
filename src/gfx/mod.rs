pub mod d3d9;
pub mod monitor;
pub mod windowed;

use crate::config::Config;
use crate::util::api::Api;

pub fn init_all(api: &Api, config: &Config) {
    windowed::init(api, config);
    monitor::init(api, config);
    d3d9::init(api, config);
}
