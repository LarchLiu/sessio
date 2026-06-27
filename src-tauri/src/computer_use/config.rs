use anyhow::Result;

use crate::computer_use::settings::ComputerUseSettings;
use crate::config;

pub fn load_settings() -> Result<ComputerUseSettings> {
    Ok(config::load_config()?.computer_use)
}

pub fn save_settings(settings: ComputerUseSettings) -> Result<ComputerUseSettings> {
    let mut app_config = config::load_config()?;
    app_config.computer_use = settings.clone();
    config::save_config(&app_config)?;
    Ok(app_config.computer_use)
}
