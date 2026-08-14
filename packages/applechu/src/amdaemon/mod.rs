//! 供游戏侧启动和 AM Daemon 侧共享的配置支持。

mod credit_freeze;
mod dns;
mod epay;
mod keychip;
mod launch;
mod netenv;
mod openssl;
mod process;
#[cfg(windows)]
mod process_args;
#[cfg(windows)]
mod process_windows;
mod runtime;

pub use credit_freeze::CreditFreezeConfig;
pub use dns::DnsConfig;
pub use epay::EpayConfig;
pub use keychip::KeychipConfig;
pub use launch::{
    append_config_args, config_files, hide_window, install_command_line_hooks,
    install_wgetmainargs_hook, INHERIT_CONSOLE_ENV,
};
pub use netenv::NetEnvConfig;
pub use openssl::OpenSslConfig;
pub use process::{auto_start, stop_auto_started};
pub use runtime::initialize;

#[cfg(test)]
pub(crate) use launch::AmdaemonConfig;
