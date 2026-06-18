pub mod amvideo;
pub mod clock;
pub mod dvd;
pub mod misc;
pub mod path_hook;
pub mod pcbid;
pub mod reg_hook;
pub mod system;
pub mod vfs;
mod winapi;

use crate::config::Config;
use crate::util::api::Api;

pub fn init_all(api: &Api, config: &Config) {
    reg_hook::init(api, config);
    api.log_info(">> reg_hook done");
    vfs::init(api, config);
    api.log_info(">> vfs done");
    amvideo::init(api, config);
    api.log_info(">> amvideo done");
    clock::init(api, config);
    api.log_info(">> clock done");
    misc::init(api, config);
    api.log_info(">> misc done");
    pcbid::init(api, config);
    api.log_info(">> pcbid done");
    dvd::init(api, config);
    api.log_info(">> dvd done");
    system::init(api, config);
    api.log_info(">> system done");
}

pub fn shutdown_all() {
    vfs::shutdown();
    pcbid::shutdown();
    misc::shutdown();
    clock::shutdown();
    amvideo::shutdown();
}
