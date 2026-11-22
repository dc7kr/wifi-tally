use crate::command::StateCommand;
use crate::domain::log::Log;
use serde_json::Value;

/// Things that happened inside the hub. Listeners: the configuration file
/// writer, the mixer driver and the socket.io forwarder.
#[derive(Clone, Debug)]
pub enum Event {
    /// Anything that is persisted has changed.
    ConfigChanged,
    /// Settings of the mixer with this id have changed.
    MixerSettingsChanged(String),
    TallyDefaultsChanged,
    ChannelsChanged,
    MixerSelectionChanged,
    MixerConnected,
    MixerDisconnected,
    ProgramChanged,
    /// A tally was created, changed or removed.
    TalliesChanged,
    TallyLogged {
        tally_id: String,
        log: Log,
    },
    /// A web tally should show a new state in all of its browsers.
    WebTallyState {
        tally_name: String,
        tally: Value,
        command: StateCommand,
    },
}
