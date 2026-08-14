#[cfg(windows)]
use super::launch::{config_files, AmdaemonConfig};
#[cfg(windows)]
use super::process_windows::{spawn_auto_started, AutoStartedChild};
use crate::util::api::API;

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
static AUTO_STARTED_CHILD: OnceLock<Mutex<Option<AutoStartedChild>>> = OnceLock::new();
#[cfg(windows)]
static TERMINATE_AUTO_STARTED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
pub fn auto_start(base_dir: &str) {
    let config = crate::config::Config::global(base_dir);
    let Some(section) = config.section::<AmdaemonConfig>() else {
        return;
    };
    if !section.enabled || !section.auto_start {
        return;
    }

    let base_dir = std::path::Path::new(base_dir).to_owned();
    let executable = section.executable.clone();
    let terminate_on_exit = section.terminate_on_exit;
    let config_files = config_files(base_dir.to_string_lossy().as_ref());
    if config_files.is_empty() {
        log_error("No AM Daemon config_*.json files were found");
        return;
    }
    TERMINATE_AUTO_STARTED.store(terminate_on_exit, Ordering::Release);

    std::thread::spawn(move || {
        let children = AUTO_STARTED_CHILD.get_or_init(|| Mutex::new(None));
        let Ok(mut guard) = children.lock() else {
            log_error("Unable to access AM Daemon child process state");
            return;
        };

        if let Some(child) = guard.as_mut() {
            match child.try_wait() {
                None => return,
                Some(_) => *guard = None,
            }
        }

        let executable_path = std::path::Path::new(&executable);
        let executable_path = if executable_path.is_absolute() {
            executable_path.to_owned()
        } else {
            base_dir.join(executable_path)
        };
        match spawn_auto_started(
            &executable_path,
            &base_dir,
            &config_files,
            terminate_on_exit,
        ) {
            Ok(child) => {
                log_info(&format!(
                    "AM Daemon started with output attached to the game console: {}{}",
                    executable_path.display(),
                    if terminate_on_exit {
                        " (job managed)"
                    } else {
                        ""
                    }
                ));
                *guard = Some(child);
            }
            Err(error) => log_error(&format!(
                "Failed to start AM Daemon: {}: {error}",
                executable_path.display()
            )),
        }
    });
}

#[cfg(not(windows))]
pub fn auto_start(_base_dir: &str) {}

#[cfg(windows)]
pub fn stop_auto_started() {
    if !TERMINATE_AUTO_STARTED.load(Ordering::Acquire) {
        return;
    }
    let Some(children) = AUTO_STARTED_CHILD.get() else {
        return;
    };
    let Ok(mut guard) = children.lock() else {
        return;
    };
    if let Some(mut child) = guard.take() {
        child.stop();
        log_info("Stopped the automatically started AM Daemon");
    }
}

#[cfg(not(windows))]
pub fn stop_auto_started() {}

fn log_info(message: &str) {
    if let Some(api) = API.get() {
        api.log_info(message);
    }
}

fn log_error(message: &str) {
    if let Some(api) = API.get() {
        api.log_error(message);
    }
}
