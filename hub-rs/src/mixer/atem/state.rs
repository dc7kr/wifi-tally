//! What the ATEM tells us about itself and the logic to derive tallies from it.
//!
//! This is a port of the parts of `atem-connection` (the library used by the
//! Node hub) that are needed for tallies: the command deserializers and
//! `listVisibleInputs`, which "emulates the ATEM's own tally logic as it appears
//! in the ATEM multiviewer".

use crate::domain::channel::Channel;
use std::collections::{BTreeMap, BTreeSet};

// Values of the enums of the protocol
const PORT_EXTERNAL: u8 = 0;
const PORT_SUPER_SOURCE: u8 = 6;
const PORT_ME_OUTPUT: u8 = 128;
const SELECTION_BACKGROUND: u8 = 1;
const STYLE_DIP: u8 = 1;
const STYLE_WIPE: u8 = 2;
const STYLE_DVE: u8 = 3;
const STYLE_STINGER: u8 = 4;
const DVE_GRAPHIC_CW_SPIN: u8 = 32;
const DVE_GRAPHIC_CCW_SPIN: u8 = 33;
const DVE_GRAPHIC_LOGO_WIPE: u8 = 34;
const KEY_TYPE_LUMA: u8 = 0;
const ART_OPTION_FOREGROUND: u8 = 1;
/// Protocol version 8.0 (the version is `major << 16 | minor`)
const PROTOCOL_V8_0: u32 = 131100;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Mode {
    Program,
    Preview,
}

#[derive(Debug, Default)]
struct Input {
    long_name: String,
    port_type: u8,
}

#[derive(Debug, Default)]
struct UpstreamKeyer {
    on_air: bool,
    key_type: u8,
    fill: u16,
    cut: u16,
}

#[derive(Debug, Default)]
struct DownstreamKeyer {
    on_air: bool,
    in_transition: bool,
    /// `None` until the properties were received
    tie: Option<bool>,
    /// fill and cut source; `None` until they were received
    sources: Option<(u16, u16)>,
}

#[derive(Debug)]
struct Dve {
    style: u8,
    fill: u16,
    key: u16,
    enable_key: bool,
}

#[derive(Debug)]
struct MixEffect {
    program: u16,
    preview: u16,
    /// bit mask, see `SELECTION_*` and `1 << (keyer + 1)`
    selection: u8,
    style: u8,
    in_transition: bool,
    handle_position: u16,
    transition_preview: bool,
    keyers: BTreeMap<u8, UpstreamKeyer>,
    dip_input: Option<u16>,
    dve: Option<Dve>,
    /// border width and input of the wipe
    wipe: Option<(u16, u16)>,
    /// media player of the stinger
    stinger_source: Option<u8>,
}

impl Default for MixEffect {
    fn default() -> Self {
        MixEffect {
            program: 0,
            preview: 0,
            selection: SELECTION_BACKGROUND,
            style: 0,
            in_transition: false,
            handle_position: 0,
            transition_preview: false,
            keyers: BTreeMap::new(),
            dip_input: None,
            dve: None,
            wipe: None,
            stinger_source: None,
        }
    }
}

#[derive(Debug, Default)]
struct SuperSource {
    /// box id → (enabled, source)
    boxes: BTreeMap<u8, (bool, u16)>,
    /// art fill source, art cut source, art option
    art: Option<(u16, u16, u8)>,
}

#[derive(Debug, Default)]
pub struct State {
    pub version: u32,
    inputs: BTreeMap<u16, Input>,
    mix_effects: BTreeMap<u8, MixEffect>,
    downstream_keyers: BTreeMap<u8, DownstreamKeyer>,
    super_sources: BTreeMap<u8, SuperSource>,
    /// the ATEM sent everything it knows (`InCm`)
    pub initialized: bool,
}

/// Reads big endian numbers; `None` if the payload is too short.
struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn u8(&self, at: usize) -> Option<u8> {
        self.0.get(at).copied()
    }
    fn u16(&self, at: usize) -> Option<u16> {
        Some(u16::from_be_bytes([*self.0.get(at)?, *self.0.get(at + 1)?]))
    }
    fn flag(&self, at: usize) -> Option<bool> {
        Some(self.u8(at)? == 1)
    }
    fn text(&self, at: usize, len: usize) -> Option<String> {
        let bytes = self.0.get(at..at + len)?;
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(len);
        Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
}

impl State {
    /// Applies one command of the ATEM. Returns `None` if the payload could not be understood;
    /// unknown commands are ignored.
    pub fn apply(&mut self, name: &str, payload: &[u8]) -> Option<()> {
        let r = Reader(payload);
        match name {
            "_ver" => {
                // the start of a new connection: forget everything
                *self = State {
                    version: u32::from_be_bytes(payload.get(0..4)?.try_into().ok()?),
                    ..State::default()
                };
            }
            "InCm" => self.initialized = true,
            "InPr" => {
                let id = r.u16(0)?;
                self.inputs.insert(
                    id,
                    Input {
                        long_name: r.text(2, 20)?,
                        port_type: r.u8(32)?,
                    },
                );
            }
            "PrgI" => self.me(r.u8(0)?).program = r.u16(2)?,
            "PrvI" => self.me(r.u8(0)?).preview = r.u16(2)?,
            "TrSS" => {
                let me = self.me(r.u8(0)?);
                me.style = r.u8(1)?;
                me.selection = r.u8(2)?;
            }
            "TrPs" => {
                let me = self.me(r.u8(0)?);
                me.in_transition = r.flag(1)?;
                me.handle_position = r.u16(4)?;
            }
            "TrPr" => self.me(r.u8(0)?).transition_preview = r.flag(1)?,
            "TDpP" => self.me(r.u8(0)?).dip_input = Some(r.u16(2)?),
            "TDvP" => {
                self.me(r.u8(0)?).dve = Some(Dve {
                    style: r.u8(3)?,
                    fill: r.u16(4)?,
                    key: r.u16(6)?,
                    enable_key: r.flag(8)?,
                })
            }
            "TWpP" => self.me(r.u8(0)?).wipe = Some((r.u16(4)?, r.u16(6)?)),
            "TStP" => self.me(r.u8(0)?).stinger_source = Some(r.u8(1)?),
            "KeOn" => {
                let on_air = r.flag(2)?;
                self.keyer(r.u8(0)?, r.u8(1)?).on_air = on_air;
            }
            "KeBP" => {
                let keyer = self.keyer(r.u8(0)?, r.u8(1)?);
                keyer.key_type = r.u8(2)?;
                keyer.fill = r.u16(6)?;
                keyer.cut = r.u16(8)?;
            }
            "DskS" => {
                let dsk = self.downstream_keyers.entry(r.u8(0)?).or_default();
                dsk.on_air = r.flag(1)?;
                dsk.in_transition = r.flag(2)?;
            }
            "DskP" => {
                self.downstream_keyers.entry(r.u8(0)?).or_default().tie = Some(r.flag(1)?);
            }
            "DskB" => {
                self.downstream_keyers.entry(r.u8(0)?).or_default().sources =
                    Some((r.u16(2)?, r.u16(4)?));
            }
            "SSrc" => {
                // since protocol 8.0 the super source is addressed as well
                let (id, art) = if self.version >= PROTOCOL_V8_0 {
                    (r.u8(0)?, (r.u16(2)?, r.u16(4)?, r.u8(6)?))
                } else {
                    (0, (r.u16(0)?, r.u16(2)?, r.u8(4)?))
                };
                self.super_sources.entry(id).or_default().art = Some(art);
            }
            "SSBP" => {
                // since protocol 8.0 the super source is addressed as well
                let (ssrc, box_id, enabled, source) = if self.version >= PROTOCOL_V8_0 {
                    (r.u8(0)?, r.u8(1)?, r.flag(2)?, r.u16(4)?)
                } else {
                    (0, r.u8(0)?, r.flag(1)?, r.u16(2)?)
                };
                self.super_sources
                    .entry(ssrc)
                    .or_default()
                    .boxes
                    .insert(box_id, (enabled, source));
            }
            _ => {}
        }
        Some(())
    }

    fn me(&mut self, id: u8) -> &mut MixEffect {
        self.mix_effects.entry(id).or_default()
    }

    fn keyer(&mut self, me: u8, keyer: u8) -> &mut UpstreamKeyer {
        self.me(me).keyers.entry(keyer).or_default()
    }

    /// The inputs of the mixer that have a physical connector, in the order of their ids.
    pub fn channels(&self) -> Vec<Channel> {
        self.inputs
            .iter()
            .filter(|(_, input)| input.port_type == PORT_EXTERNAL)
            .map(|(id, input)| Channel::new(id.to_string(), Some(input.long_name.clone())))
            .collect()
    }

    /// All sources that are visible on the program or preview output of a mix effect, like
    /// the multiviewer of the ATEM shows them: sources of keyers, transitions, super sources
    /// and nested mix effects are included. Sorted.
    pub fn visible_inputs(&self, mode: Mode, me: u8) -> Vec<u16> {
        let mut inputs = BTreeSet::new();
        self.add_mix_effect(&mut inputs, mode, me);

        // look into inputs that show other sources, until nothing new is found
        let mut done = BTreeSet::new();
        loop {
            let todo: Vec<u32> = inputs.difference(&done).copied().collect();
            if todo.is_empty() {
                break;
            }
            for id in todo {
                done.insert(id);
                let Some(input) = u16::try_from(id).ok().and_then(|id| self.inputs.get(&id))
                else {
                    continue; // not known (yet)
                };
                match input.port_type {
                    PORT_SUPER_SOURCE => {
                        let Some(ssrc) = id
                            .checked_sub(6000)
                            .and_then(|i| u8::try_from(i).ok())
                            .and_then(|i| self.super_sources.get(&i))
                        else {
                            continue;
                        };
                        for (enabled, source) in ssrc.boxes.values() {
                            if *enabled {
                                inputs.insert(*source as u32);
                            }
                        }
                        if let Some((fill, cut, option)) = ssrc.art {
                            inputs.insert(fill as u32);
                            if option == ART_OPTION_FOREGROUND {
                                inputs.insert(cut as u32);
                            }
                        }
                    }
                    PORT_ME_OUTPUT => {
                        // 10010 is the program of ME 1, 10011 its preview, 10020 the program of ME 2, …
                        let Some(rest) = id.checked_sub(10000) else {
                            continue;
                        };
                        let Some(nested) = (rest / 10).checked_sub(1).and_then(|n| u8::try_from(n).ok())
                        else {
                            continue;
                        };
                        let mode = if rest % 10 == 0 {
                            Mode::Program
                        } else {
                            Mode::Preview
                        };
                        self.add_mix_effect(&mut inputs, mode, nested);
                    }
                    _ => {}
                }
            }
        }
        inputs.into_iter().filter_map(|i| u16::try_from(i).ok()).collect()
    }

    fn add_mix_effect(&self, inputs: &mut BTreeSet<u32>, mode: Mode, me_id: u8) {
        let default = MixEffect::default();
        let me = self.mix_effects.get(&me_id).unwrap_or(&default);
        let mut add = |i: u16| {
            inputs.insert(i as u32);
        };

        match mode {
            Mode::Preview => {
                if me.selection & SELECTION_BACKGROUND != 0 {
                    add(me.preview);
                }
            }
            Mode::Program => add(me.program),
        }

        // upstream keyers
        for (id, keyer) in &me.keyers {
            let part_of_transition = me.selection & (1 << (id + 1)) != 0;
            let visible = match mode {
                Mode::Program if me.in_transition => keyer.on_air || part_of_transition,
                Mode::Program => keyer.on_air,
                Mode::Preview => {
                    (part_of_transition && !keyer.on_air) || (keyer.on_air && !part_of_transition)
                }
            };
            if visible {
                add(keyer.fill);
                // the only type of key that uses the cut source
                if keyer.key_type == KEY_TYPE_LUMA {
                    add(keyer.cut);
                }
            }
        }

        // downstream keyers belong to the first mix effect
        if me_id == 0 {
            for dsk in self.downstream_keyers.values() {
                let visible = match mode {
                    Mode::Program => dsk.on_air || dsk.in_transition,
                    Mode::Preview => dsk.tie == Some(true) && !dsk.on_air,
                };
                if let (true, Some((fill, cut))) = (visible, dsk.sources) {
                    add(fill);
                    add(cut);
                }
            }
        }

        // what takes part in a running transition
        let transitioning = match mode {
            Mode::Program => me.in_transition,
            Mode::Preview => me.transition_preview && me.handle_position > 0,
        };
        if !transitioning {
            return;
        }
        if me.selection & SELECTION_BACKGROUND != 0 {
            add(me.preview);
        }
        match me.style {
            STYLE_DIP => {
                if let Some(input) = me.dip_input {
                    add(input);
                }
            }
            STYLE_DVE => {
                if let Some(dve) = &me.dve {
                    if matches!(
                        dve.style,
                        DVE_GRAPHIC_CW_SPIN | DVE_GRAPHIC_CCW_SPIN | DVE_GRAPHIC_LOGO_WIPE
                    ) {
                        add(dve.fill);
                        if dve.enable_key {
                            add(dve.key);
                        }
                    }
                }
            }
            STYLE_WIPE => {
                if let Some((width, input)) = me.wipe {
                    if width > 0 {
                        add(input);
                    }
                }
            }
            STYLE_STINGER => {
                if let Some(player) = me.stinger_source {
                    let fill = 3000 + player as u16 * 10;
                    add(fill);
                    add(fill + 1); // the key
                }
            }
            _ => {} // a mix only needs the preview input
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(state: &mut State, name: &str, payload: &[u8]) {
        state.apply(name, payload).unwrap_or_else(|| panic!("{name} was rejected"));
    }

    fn input(state: &mut State, id: u16, name: &str, port_type: u8) {
        let mut p = vec![0u8; 36];
        p[0..2].copy_from_slice(&id.to_be_bytes());
        p[2..2 + name.len()].copy_from_slice(name.as_bytes());
        p[32] = port_type;
        apply(state, "InPr", &p);
    }

    fn pgm(state: &mut State, me: u8, source: u16) {
        let s = source.to_be_bytes();
        apply(state, "PrgI", &[me, 0, s[0], s[1]]);
    }

    fn pvw(state: &mut State, me: u8, source: u16) {
        let s = source.to_be_bytes();
        apply(state, "PrvI", &[me, 0, s[0], s[1]]);
    }

    /// bytes 6/7 fill, 8/9 cut
    fn keyer_props(state: &mut State, me: u8, keyer: u8, key_type: u8, fill: u16, cut: u16) {
        let mut p = vec![0u8; 20];
        p[0] = me;
        p[1] = keyer;
        p[2] = key_type;
        p[6..8].copy_from_slice(&fill.to_be_bytes());
        p[8..10].copy_from_slice(&cut.to_be_bytes());
        apply(state, "KeBP", &p);
    }

    #[test]
    fn program_and_preview_of_the_first_mix_effect() {
        let mut s = State::default();
        pgm(&mut s, 0, 2);
        pvw(&mut s, 0, 3);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![2]);
        assert_eq!(s.visible_inputs(Mode::Preview, 0), vec![3]);
    }

    #[test]
    fn preview_is_empty_if_the_background_does_not_take_part_in_the_next_transition() {
        let mut s = State::default();
        pvw(&mut s, 0, 3);
        // style 0 (mix), selection: only key 1
        apply(&mut s, "TrSS", &[0, 0, 0b10, 0, 0b10]);
        assert_eq!(s.visible_inputs(Mode::Preview, 0), Vec::<u16>::new());
    }

    #[test]
    fn upstream_keyers() {
        let mut s = State::default();
        pgm(&mut s, 0, 1);
        pvw(&mut s, 0, 2);
        keyer_props(&mut s, 0, 0, 0, 5, 6); // luma key: fill 5 and cut 6
        keyer_props(&mut s, 0, 1, 2, 7, 8); // pattern key: only the fill counts

        // off air, but not part of the transition
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1]);
        assert_eq!(s.visible_inputs(Mode::Preview, 0), vec![2]);

        // key 1 is on air
        apply(&mut s, "KeOn", &[0, 0, 1]);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 5, 6]);
        assert_eq!(s.visible_inputs(Mode::Preview, 0), vec![2, 5, 6]);

        // key 2 is going on air with the next transition (selection: background and key 2)
        apply(&mut s, "TrSS", &[0, 0, 0b101, 0, 0b101]);
        assert_eq!(s.visible_inputs(Mode::Preview, 0), vec![2, 5, 6, 7]);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 5, 6]);

        // during the transition both are on program
        apply(&mut s, "TrPs", &[0, 1, 20, 0, 0x10, 0x00, 0, 0]);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 2, 5, 6, 7]);
    }

    #[test]
    fn downstream_keyers_belong_to_the_first_mix_effect_only() {
        let mut s = State::default();
        pgm(&mut s, 0, 1);
        pgm(&mut s, 1, 1);
        apply(&mut s, "DskB", &[0, 0, 0, 9, 0, 10, 0, 0]);
        apply(&mut s, "DskS", &[0, 1, 0, 0, 0]); // on air
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 9, 10]);
        assert_eq!(s.visible_inputs(Mode::Program, 1), vec![1]);
    }

    #[test]
    fn tied_downstream_keyer_shows_in_preview_until_it_is_on_air() {
        let mut s = State::default();
        pvw(&mut s, 0, 2);
        apply(&mut s, "DskB", &[0, 0, 0, 9, 0, 10, 0, 0]);
        apply(&mut s, "DskP", &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        apply(&mut s, "DskS", &[0, 0, 0, 0, 0]);
        assert_eq!(s.visible_inputs(Mode::Preview, 0), vec![2, 9, 10]);
        apply(&mut s, "DskS", &[0, 1, 0, 0, 0]);
        assert_eq!(s.visible_inputs(Mode::Preview, 0), vec![2]);
    }

    #[test]
    fn downstream_keyer_without_sources_is_ignored() {
        let mut s = State::default();
        apply(&mut s, "DskS", &[0, 1, 0, 0, 0]);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![0]);
    }

    #[test]
    fn transition_styles() {
        let mut s = State::default();
        pgm(&mut s, 0, 1);
        pvw(&mut s, 0, 2);
        apply(&mut s, "TrPs", &[0, 1, 10, 0, 0, 100, 0, 0]);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 2], "mix");

        // dip with input 11
        apply(&mut s, "TrSS", &[0, 1, 1, 1, 1]);
        apply(&mut s, "TDpP", &[0, 30, 0, 11]);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 2, 11], "dip");

        // wipe: a border of width 0 does not show the border input
        apply(&mut s, "TrSS", &[0, 2, 1, 2, 1]);
        let mut wipe = vec![0u8; 20];
        wipe[6..8].copy_from_slice(&12u16.to_be_bytes());
        apply(&mut s, "TWpP", &wipe);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 2], "wipe without border");
        wipe[4..6].copy_from_slice(&5u16.to_be_bytes());
        apply(&mut s, "TWpP", &wipe);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 2, 12], "wipe");

        // stinger with media player 2 → 3010 and 3011
        apply(&mut s, "TrSS", &[0, 4, 1, 4, 1]);
        apply(&mut s, "TStP", &[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 2, 3010, 3011], "stinger");

        // dve: only the graphic styles use their sources
        apply(&mut s, "TrSS", &[0, 3, 1, 3, 1]);
        let dve = |style: u8, enable_key: u8| {
            let mut p = vec![0u8; 20];
            p[3] = style;
            p[4..6].copy_from_slice(&13u16.to_be_bytes());
            p[6..8].copy_from_slice(&14u16.to_be_bytes());
            p[8] = enable_key;
            p
        };
        apply(&mut s, "TDvP", &dve(1, 1)); // swoosh
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 2], "dve swoosh");
        apply(&mut s, "TDvP", &dve(34, 0)); // logo wipe without key
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 2, 13], "dve logo");
        apply(&mut s, "TDvP", &dve(32, 1));
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![1, 2, 13, 14], "dve logo with key");
    }

    #[test]
    fn transition_preview_needs_a_moved_handle() {
        let mut s = State::default();
        pvw(&mut s, 0, 2);
        apply(&mut s, "TrSS", &[0, 1, 1, 1, 1]);
        apply(&mut s, "TDpP", &[0, 30, 0, 11]);
        apply(&mut s, "TrPr", &[0, 1]);
        assert_eq!(s.visible_inputs(Mode::Preview, 0), vec![2]);
        apply(&mut s, "TrPs", &[0, 0, 0, 0, 0x03, 0xe8, 0, 0]);
        assert_eq!(s.visible_inputs(Mode::Preview, 0), vec![2, 11]);
    }

    #[test]
    fn super_source_shows_its_boxes() {
        let mut s = State::default();
        input(&mut s, 6000, "SuperSource", 6);
        pgm(&mut s, 0, 6000);
        // protocol 8: ssrc 0, box 1, enabled, source 4
        s.version = PROTOCOL_V8_0;
        apply(&mut s, "SSBP", &[0, 1, 1, 0, 0, 4, 0, 0, 0, 0]);
        apply(&mut s, "SSBP", &[0, 2, 0, 0, 0, 5, 0, 0, 0, 0]); // disabled
        let mut ssrc = vec![0u8; 36];
        ssrc[2..4].copy_from_slice(&15u16.to_be_bytes());
        ssrc[4..6].copy_from_slice(&16u16.to_be_bytes());
        ssrc[6] = 0; // background art: the cut source does not count
        apply(&mut s, "SSrc", &ssrc);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![4, 15, 6000]);
        ssrc[6] = 1;
        apply(&mut s, "SSrc", &ssrc);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![4, 15, 16, 6000]);
    }

    #[test]
    fn super_source_boxes_before_protocol_8() {
        let mut s = State::default();
        input(&mut s, 6000, "SuperSource", 6);
        pgm(&mut s, 0, 6000);
        s.version = 131099;
        // box 3, enabled, source 7
        apply(&mut s, "SSBP", &[3, 1, 0, 7, 0, 0, 0, 0]);
        // art: fill 15, cut 16, foreground
        apply(&mut s, "SSrc", &[0, 15, 0, 16, 1, 0, 0, 0]);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![7, 15, 16, 6000]);
    }

    #[test]
    fn nested_mix_effect_outputs() {
        let mut s = State::default();
        input(&mut s, 10020, "ME 2 PGM", 128);
        input(&mut s, 10021, "ME 2 PVW", 128);
        pgm(&mut s, 0, 10020);
        pgm(&mut s, 1, 5);
        pvw(&mut s, 1, 6);
        pvw(&mut s, 0, 10021);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![5, 10020]);
        assert_eq!(s.visible_inputs(Mode::Preview, 0), vec![6, 10021]);
    }

    #[test]
    fn mix_effects_that_show_each_other_do_not_loop_forever() {
        let mut s = State::default();
        input(&mut s, 10010, "ME 1", 128);
        input(&mut s, 10020, "ME 2", 128);
        pgm(&mut s, 0, 10020);
        pgm(&mut s, 1, 10010);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![10010, 10020]);
    }

    #[test]
    fn only_external_inputs_are_channels() {
        let mut s = State::default();
        input(&mut s, 2, "Camera 2", 0);
        input(&mut s, 0, "Black", 1);
        input(&mut s, 1, "Camera 1", 0);
        input(&mut s, 1000, "Bars", 2);
        assert_eq!(
            s.channels(),
            vec![
                Channel::new("1", Some("Camera 1".into())),
                Channel::new("2", Some("Camera 2".into())),
            ]
        );
    }

    #[test]
    fn a_new_version_command_resets_everything() {
        let mut s = State::default();
        pgm(&mut s, 0, 4);
        apply(&mut s, "InCm", &[]);
        assert!(s.initialized);
        apply(&mut s, "_ver", &[0, 2, 0, 28]);
        assert!(!s.initialized);
        assert_eq!(s.version, 131100);
        assert_eq!(s.visible_inputs(Mode::Program, 0), vec![0]);
    }

    #[test]
    fn short_or_unknown_commands_are_harmless() {
        let mut s = State::default();
        assert!(s.apply("PrgI", &[0, 0]).is_none());
        assert!(s.apply("InPr", &[0, 1, 2]).is_none());
        assert!(s.apply("????", &[1, 2, 3]).is_some());
    }
}
