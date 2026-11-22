pub mod atem;
pub mod config;
pub mod connectors;
pub mod driver;
pub mod obs;
pub mod vmix;

use config::*;

/// Ids of the mixers a user may select. The order matters: the first entries act as defaults.
pub fn allowed_mixers(is_dev: bool, is_test: bool) -> Vec<&'static str> {
    [MOCK, TEST, NULL, ATEM, OBS, ROLAND_V8HD, ROLAND_V60HD, VMIX]
        .into_iter()
        .filter(|id| (*id != MOCK || is_dev) && (*id != TEST || is_test))
        .collect()
}
