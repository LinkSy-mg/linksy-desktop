//! The microphone this computer offers to other apps. A PipeWire node published with
//! `pipewiresink mode=provide` stops delivering after a fraction of a second on PipeWire 1.6
//! (desktop#89, PipeWire's own issue 5378). So the phone's audio is played into a null sink,
//! an ordinary one, and a microphone is made from that sink's monitor, both through
//! PipeWire's PulseAudio layer.

use std::process::Stdio;

/// The microphone's name, as apps have always found it.
const NODE: &str = "linksy-mic";
/// The sink the audio is played into.
const SINK: &str = "linksy-mic-sink";

/// The sink that plays into it. Without `node.dont-fallback` a missing target means the
/// default sink, and the phone's voice would come out of the speakers.
pub fn feed() -> String {
    format!("pipewiresink sync=false target-object={SINK} stream-properties=\"props,node.name={NODE}-feed,node.dont-fallback=true\"")
}

/// A microphone made from a null sink, both removed again when this goes.
pub struct VirtualMic {
    source: String,
    sink: String,
}

impl VirtualMic {
    pub fn create(description: &str) -> Result<Self, String> {
        // Ones an earlier run left behind (it was killed) would take the names
        if let Ok(modules) = pactl(&["list", "short", "modules"]) {
            for id in leftovers(&modules) {
                let _ = pactl(&["unload-module", id]);
            }
        }
        let description = plain(description);
        let sink = pactl(&[
            "load-module",
            "module-null-sink",
            &format!("sink_name={SINK}"),
            "channel_map=mono",
            "rate=48000",
            "format=s16le",
            &format!("sink_properties=\"device.description='{description} (audio in)'\""),
        ])?;
        let source = pactl(&[
            "load-module",
            "module-remap-source",
            &format!("master={SINK}.monitor"),
            &format!("source_name={NODE}"),
            &format!("source_properties=\"device.description='{description}'\""),
        ])
        .inspect_err(|_| {
            let _ = pactl(&["unload-module", &sink]);
        })?;
        Ok(Self { source, sink })
    }
}

impl Drop for VirtualMic {
    fn drop(&mut self) {
        let _ = pactl(&["unload-module", &self.source]);
        let _ = pactl(&["unload-module", &self.sink]);
    }
}

/// `pactl` where the sound server is: on the computer, from the Flatpak too.
fn pactl(args: &[&str]) -> Result<String, String> {
    let out = linksy_host::command("pactl", &[])
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("pactl: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!("pactl {}: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// The ids of the modules in `pactl list short modules` that made a microphone of ours, the
/// source first: it is removed before the sink it listens to.
fn leftovers(modules: &str) -> Vec<&str> {
    let find = |module: &str, arg: String| -> Vec<&str> {
        modules
            .lines()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let (id, name, args) = (fields.next()?, fields.next()?, fields.next().unwrap_or_default());
                (name == module && args.split(' ').any(|a| a == arg)).then_some(id)
            })
            .collect()
    };
    let mut ids = find("module-remap-source", format!("source_name={NODE}"));
    ids.extend(find("module-null-sink", format!("sink_name={SINK}")));
    ids
}

/// A name that can sit inside the quotes of a module argument.
fn plain(name: &str) -> String {
    name.chars().filter(|c| !c.is_control() && !matches!(c, '"' | '\'' | '\\' | ',')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_an_earlier_run_left_behind_is_found_source_first() {
        let list = "536870916\tmodule-null-sink\tsink_name=linksy-mic-sink channel_map=mono rate=48000 format=s16le sink_properties=\"device.description='Linksy Microphone (Pixel) (audio in)'\"\t\n\
                    536870917\tmodule-remap-source\tmaster=linksy-mic-sink.monitor source_name=linksy-mic source_properties=\"device.description='Linksy Microphone (Pixel)'\"\t\n\
                    536870918\tmodule-null-sink\tsink_name=other\t\n\
                    536870919\tmodule-loopback\tsink_name=linksy-mic-sink\t\n\
                    27\tmodule-switch-on-connect\t\t";
        assert_eq!(leftovers(list), ["536870917", "536870916"]);
        assert!(leftovers("").is_empty());
    }

    #[test]
    fn a_device_name_cannot_break_out_of_its_quotes() {
        assert_eq!(plain("Linksy Microphone (Pixel 8)"), "Linksy Microphone (Pixel 8)");
        assert_eq!(plain("Bob's \"phone\", x\\y\n"), "Bobs phone xy");
    }
}
