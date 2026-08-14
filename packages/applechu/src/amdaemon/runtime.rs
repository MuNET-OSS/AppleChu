use crate::config::DiagnosticLevel;
use crate::util::api::{Api, StandaloneLogger, API};

pub fn initialize(
    base_dir: &str,
    logger: StandaloneLogger,
    module_order: &[&str],
) -> Result<(), String> {
    let api =
        Api::standalone(logger).ok_or_else(|| "failed to inspect AM Daemon PE image".to_owned())?;
    api.install();
    let api = API
        .get()
        .ok_or_else(|| "failed to install standalone API".to_owned())?;
    let config = crate::config::Config::global(base_dir);

    for diagnostic in config.diagnostics() {
        match diagnostic.level {
            DiagnosticLevel::Warning => api.log_warn(&diagnostic.message),
            DiagnosticLevel::Error => api.log_error(&diagnostic.message),
        }
    }
    if !config.is_valid() {
        return Err("AppleChu.toml is invalid".to_owned());
    }

    crate::module_registry::init_ordered(api, config, module_order);
    Ok(())
}
