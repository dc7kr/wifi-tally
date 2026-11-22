//! Compares the tally logic with the reference implementation, the `atem-connection` library of the
//! Node hub. `tests/atem-fixtures.js` replays random command sequences through the library and
//! records what it reports; this test feeds the same bytes to the Rust code.

use serde_json::Value;
use vtally_hub::mixer::atem::state::{Mode, State};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

fn numbers(v: &Value) -> Vec<u16> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|n| n.as_u64().unwrap() as u16)
        .collect()
}

#[test]
fn matches_the_atem_connection_library() {
    let fixtures: Value =
        serde_json::from_str(include_str!("fixtures/atem-visible-inputs.json")).unwrap();
    let cases = fixtures["cases"].as_array().unwrap();
    assert!(cases.len() >= 100);

    for (n, case) in cases.iter().enumerate() {
        let mut state = State::default();
        let version = (case["version"].as_u64().unwrap() as u32).to_be_bytes();
        state.apply("_ver", &version).unwrap();

        let commands = case["commands"].as_array().unwrap();
        for command in commands {
            let name = command[0].as_str().unwrap();
            let payload = hex(command[1].as_str().unwrap());
            assert!(
                state.apply(name, &payload).is_some(),
                "case {n}: {name} was rejected"
            );
        }

        for me in 0..2u8 {
            for (mode, key) in [(Mode::Program, "program"), (Mode::Preview, "preview")] {
                let expected = numbers(&case[key][me as usize]);
                assert_eq!(
                    state.visible_inputs(mode, me),
                    expected,
                    "case {n}: {key} of ME {me} after {commands:?}"
                );
            }
        }

        let channels: Vec<(String, String)> = state
            .channels()
            .into_iter()
            .map(|c| (c.id, c.name.unwrap()))
            .collect();
        let expected: Vec<(String, String)> = case["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| (c[0].to_string(), c[1].as_str().unwrap().to_string()))
            .collect();
        assert_eq!(channels, expected, "case {n}: channels");
    }
}
