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
use crate::util::memory::PatchMemory;

pub fn apply_pre_tls<M: PatchMemory>(memory: &M, config: &Config) {
    network::apply_early(memory, config);
    custom_version::apply_early(memory, config);
    skip_startup::apply_early(memory, config);
    free_play::apply_early(memory, config);
    timers::apply_early(memory, config);
    skip_map_anim::apply_early(memory, config);
    unlock_tracks::apply_early(memory, config);
    unlock_120fps::apply_early(memory, config);
    fast_restart::apply_early(memory, config);
    bypass::apply_early(memory, config);
    audio::apply_early(memory, config);
    custom_freeplay::apply_early(memory, config);
}

pub fn install_pre_entry_hooks(api: &Api, config: &Config) {
    network::install_pre_entry_hook(api, config);
    net_log::install_pre_entry_hook(api, config);
}
