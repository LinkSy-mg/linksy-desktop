//! The keys that, held together, switch Handover on and off.

use std::collections::BTreeSet;

/// Both Ctrl keys at once: it types nothing, and no keyboard lacks them.
pub const DEFAULT: &str = "leftctrl+rightctrl";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    /// Every group needs one of its keys down.
    groups: Vec<Vec<u16>>,
}

const NAMES: &[(&str, &[u16])] = &[
    ("leftctrl", &[29]),
    ("rightctrl", &[97]),
    ("ctrl", &[29, 97]),
    ("leftalt", &[56]),
    ("rightalt", &[100]),
    ("alt", &[56, 100]),
    ("leftshift", &[42]),
    ("rightshift", &[54]),
    ("shift", &[42, 54]),
    ("leftmeta", &[125]),
    ("rightmeta", &[126]),
    ("meta", &[125, 126]),
    ("scrolllock", &[70]),
    ("pause", &[119]),
];

impl Chord {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut groups = Vec::new();
        for name in text.split('+').map(|n| n.trim().to_ascii_lowercase()) {
            let keys = match NAMES.iter().find(|(n, _)| *n == name) {
                Some((_, keys)) => keys.to_vec(),
                None => vec![function_key(&name).ok_or_else(|| format!("unknown key \"{name}\""))?],
            };
            groups.push(keys);
        }
        if groups.is_empty() || groups.iter().any(Vec::is_empty) {
            return Err("empty hotkey".into());
        }
        if groups.len() > 4 {
            return Err("a hotkey is at most 4 keys".into());
        }
        Ok(Self { groups })
    }

    pub fn satisfied(&self, down: &BTreeSet<u16>) -> bool {
        self.groups.iter().all(|group| group.iter().any(|k| down.contains(k)))
    }
}

fn function_key(name: &str) -> Option<u16> {
    match name.strip_prefix('f')?.parse::<u16>().ok()? {
        n @ 1..=10 => Some(58 + n),
        11 => Some(87),
        12 => Some(88),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn down(keys: &[u16]) -> BTreeSet<u16> {
        keys.iter().copied().collect()
    }

    #[test]
    fn the_default_is_both_ctrl_keys() {
        let chord = Chord::parse(DEFAULT).unwrap();
        assert!(!chord.satisfied(&down(&[29])));
        assert!(!chord.satisfied(&down(&[97])));
        assert!(chord.satisfied(&down(&[29, 97])));
        assert!(chord.satisfied(&down(&[29, 97, 30])));
    }

    #[test]
    fn a_name_without_a_side_takes_either() {
        let chord = Chord::parse("ctrl+alt+f12").unwrap();
        assert!(chord.satisfied(&down(&[97, 56, 88])));
        assert!(chord.satisfied(&down(&[29, 100, 88])));
        assert!(!chord.satisfied(&down(&[29, 56])));
    }

    #[test]
    fn bad_hotkeys_say_why() {
        assert!(Chord::parse("ctrl+banana").unwrap_err().contains("banana"));
        assert!(Chord::parse("").is_err());
        assert!(Chord::parse("f13").is_err());
        assert!(Chord::parse("ctrl+alt+shift+meta+f1").is_err());
    }
}
