pub mod audio;
pub mod bypass;
pub mod custom_freeplay;
pub mod custom_version;
pub mod fast_restart;
pub mod free_play;
pub mod net_log;
pub mod network;
pub mod skip_map_anim;
pub mod skip_startup;
pub mod timers;
pub mod unlock_120fps;
pub mod unlock_tracks;

use crate::config::Config;
use crate::util::api::Api;

pub fn apply_all(api: &Api, config: &Config) {
    custom_version::apply(api, config);
    skip_startup::apply(api, config);
    free_play::apply(api, config);
    timers::apply(api, config);
    skip_map_anim::apply(api, config);
    unlock_tracks::apply(api, config);
    unlock_120fps::apply(api, config);
    fast_restart::apply(api, config);
    network::apply(api, config);
    bypass::apply(api, config);
    audio::apply(api, config);
    custom_freeplay::apply(api, config);
    net_log::apply(api, config);
}
