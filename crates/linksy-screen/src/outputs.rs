//! The desktop's outputs as Wayland describes them (xdg-output), for KDE's second display:
//! its stream's size is in pixels, but KDE places pointer positions in the desktop's own
//! coordinates, which are smaller when the display is scaled (125 %: a tap meant for the
//! middle landed lower right, desktop#51). GNOME takes positions in the stream's pixels.

use std::sync::{Arc, Mutex};

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::{self, ZxdgOutputManagerV1};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};

/// The second display's size where the pointer goes, changed while it's shared.
pub type Size = Arc<Mutex<Option<(f64, f64)>>>;

#[derive(Default, Debug)]
struct Output {
    /// The compositor's number for it: a newer output has a bigger one.
    id: u32,
    name: Option<String>,
    /// Its current mode, in pixels.
    mode: Option<(i32, i32)>,
    /// Its size on the desktop.
    logical: Option<(i32, i32)>,
}

#[derive(Default)]
struct Outputs {
    list: Vec<Output>,
    manager: Option<ZxdgOutputManagerV1>,
    changed: bool,
}

impl Outputs {
    fn add(&mut self, registry: &WlRegistry, id: u32, version: u32, qh: &QueueHandle<Self>) {
        let i = self.list.len();
        let output: WlOutput = registry.bind(id, version.min(2), qh, i);
        if let Some(manager) = &self.manager {
            manager.get_xdg_output(&output, qh, i);
        }
        self.list.push(Output { id, ..Default::default() });
    }
}

/// KDE's second display, `name` (the stream's mapping id) and `pixels` big: its size on the
/// desktop goes in `size`, now and whenever KDE changes it (it can scale the display after
/// making it). What it finds goes in the logs.
pub fn follow_kde_display(name: Option<String>, pixels: (f64, f64), size: Size) {
    // Another desktop's display
    if name.as_deref().is_some_and(|n| !n.contains("virtual-xdp-kde")) {
        return;
    }
    let kde = name.is_some();
    let conn = match connect() {
        Ok(c) => c,
        Err(e) => {
            if kde {
                eprintln!("linksy-screen: can't reach the desktop's Wayland ({e}); taps on the second display assume it isn't scaled");
            }
            return;
        }
    };
    let Ok((globals, mut queue)) = registry_queue_init::<Outputs>(&conn) else { return };
    let qh = queue.handle();
    let mut state = Outputs { manager: globals.bind(&qh, 2..=3, ()).ok(), ..Default::default() };
    if state.manager.is_none() {
        if kde {
            eprintln!("linksy-screen: the desktop doesn't describe its outputs (xdg-output); taps on the second display assume it isn't scaled");
        }
        return;
    }
    for global in globals.contents().clone_list().into_iter().filter(|g| g.interface == "wl_output") {
        state.add(globals.registry(), global.name, global.version, &qh);
    }
    if queue.roundtrip(&mut state).is_err() {
        return;
    }
    let mut last = pick(&state.list, name.as_deref(), pixels);
    say(last, pixels, &state.list, name.as_deref());
    set(&size, last, pixels);
    if !kde {
        return;
    }
    std::thread::spawn(move || {
        let _conn = conn;
        while queue.blocking_dispatch(&mut state).is_ok() {
            if !std::mem::take(&mut state.changed) {
                continue;
            }
            let now = pick(&state.list, name.as_deref(), pixels);
            if now.is_some() && now != last {
                say(now, pixels, &state.list, name.as_deref());
                set(&size, now, pixels);
                last = now;
            }
        }
    });
}

/// The desktop's Wayland: a service started before the desktop said where it is looks in
/// the usual place.
pub(crate) fn connect() -> Result<Connection, String> {
    Connection::connect_to_env().or_else(|e| {
        let dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| e.to_string())?;
        let socket = std::os::unix::net::UnixStream::connect(std::path::Path::new(&dir).join("wayland-0")).map_err(|_| e.to_string())?;
        Connection::from_socket(socket).map_err(|e| e.to_string())
    })
}

fn set(size: &Size, desktop: Option<(f64, f64)>, pixels: (f64, f64)) {
    *size.lock().unwrap() = Some(desktop.unwrap_or(pixels));
}

fn say(desktop: Option<(f64, f64)>, pixels: (f64, f64), outputs: &[Output], name: Option<&str>) {
    match desktop {
        Some(d) if d != pixels => eprintln!("linksy-screen: the second display is {}x{} pixels, {}x{} on the desktop (scaled); the phone's taps follow the desktop's", pixels.0, pixels.1, d.0, d.1),
        Some(_) => eprintln!("linksy-screen: the second display is {}x{}, not scaled", pixels.0, pixels.1),
        None if name.is_some() => {
            let seen: Vec<String> = outputs.iter().filter_map(|o| Some(format!("{} {:?}/{:?}", o.name.as_deref()?, o.mode?, o.logical?))).collect();
            eprintln!("linksy-screen: no output named {} yet (outputs: {}); taps assume it isn't scaled", name.unwrap_or_default(), seen.join(", "));
        }
        None => {}
    }
}

/// The size on the desktop of KDE's second display: the output named `name`, or without
/// one the newest output KDE made for sharing (`Virtual-virtual-xdp-kde-…`) `size` pixels
/// big.
fn pick(outputs: &[Output], name: Option<&str>, (w, h): (f64, f64)) -> Option<(f64, f64)> {
    let mode = (w.round() as i32, h.round() as i32);
    outputs
        .iter()
        .filter(|o| match (name, o.name.as_deref()) {
            (Some(want), Some(n)) => n == want,
            (None, Some(n)) => n.contains("virtual-xdp-kde") && o.mode == Some(mode),
            _ => false,
        })
        .max_by_key(|o| o.id)
        .and_then(|o| o.logical)
        .filter(|&(w, h)| w > 0 && h > 0)
        .map(|(w, h)| (w as f64, h as f64))
}

impl Dispatch<WlRegistry, GlobalListContents> for Outputs {
    fn event(state: &mut Self, registry: &WlRegistry, event: wl_registry::Event, _: &GlobalListContents, _: &Connection, qh: &QueueHandle<Self>) {
        match event {
            wl_registry::Event::Global { name, interface, version } if interface == WlOutput::interface().name => state.add(registry, name, version, qh),
            wl_registry::Event::GlobalRemove { name } => {
                if let Some(o) = state.list.iter_mut().find(|o| o.id == name) {
                    o.name = None;
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputManagerV1, ()> for Outputs {
    fn event(_: &mut Self, _: &ZxdgOutputManagerV1, _: zxdg_output_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlOutput, usize> for Outputs {
    fn event(state: &mut Self, _: &WlOutput, event: wl_output::Event, &i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            wl_output::Event::Mode { flags: WEnum::Value(flags), width, height, .. } if flags.contains(wl_output::Mode::Current) => state.list[i].mode = Some((width, height)),
            wl_output::Event::Done => state.changed = true,
            _ => {}
        }
    }
}

impl Dispatch<ZxdgOutputV1, usize> for Outputs {
    fn event(state: &mut Self, _: &ZxdgOutputV1, event: zxdg_output_v1::Event, &i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zxdg_output_v1::Event::LogicalSize { width, height } => state.list[i].logical = Some((width, height)),
            zxdg_output_v1::Event::Name { name } => state.list[i].name = Some(name),
            zxdg_output_v1::Event::Done => state.changed = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: u32, name: &str, mode: (i32, i32), logical: (i32, i32)) -> Output {
        Output { id, name: Some(name.into()), mode: Some(mode), logical: Some(logical) }
    }

    #[test]
    fn kde_second_display_scaled_to_125() {
        let outputs = [
            output(40, "eDP-1", (1920, 1080), (1536, 864)),
            output(61, "Virtual-virtual-xdp-kde-", (1920, 1080), (1536, 864)),
        ];
        assert_eq!(pick(&outputs, None, (1920.0, 1080.0)), Some((1536.0, 864.0)));
        assert_eq!(pick(&outputs, Some("Virtual-virtual-xdp-kde-"), (1920.0, 1080.0)), Some((1536.0, 864.0)));
    }

    #[test]
    fn the_newest_of_kdes_displays_in_that_size() {
        let outputs = [
            output(61, "Virtual-virtual-xdp-kde-", (1920, 1080), (1920, 1080)),
            output(75, "Virtual-virtual-xdp-kde--1", (1920, 1080), (1536, 864)),
            output(80, "Virtual-virtual-xdp-kde--2", (2400, 1080), (2400, 1080)),
        ];
        assert_eq!(pick(&outputs, None, (1920.0, 1080.0)), Some((1536.0, 864.0)));
    }

    #[test]
    fn by_name_whatever_its_mode() {
        // The desktop gave it another mode than the stream says: the name still finds it
        let outputs = [
            output(61, "Virtual-virtual-xdp-kde-", (1920, 1080), (1920, 1080)),
            output(75, "Virtual-virtual-xdp-kde--1", (2560, 1440), (2048, 1152)),
        ];
        assert_eq!(pick(&outputs, Some("Virtual-virtual-xdp-kde--1"), (1920.0, 1080.0)), Some((2048.0, 1152.0)));
        assert_eq!(pick(&outputs, Some("Virtual-virtual-xdp-kde--2"), (1920.0, 1080.0)), None);
    }

    #[test]
    fn nothing_but_kdes_own_displays() {
        // GNOME's virtual monitor, or a monitor in that size: the stream's size stays
        let outputs = [output(40, "Meta-0", (1920, 1080), (960, 540)), output(41, "eDP-1", (1920, 1080), (1536, 864))];
        assert_eq!(pick(&outputs, None, (1920.0, 1080.0)), None);
        assert_eq!(pick(&[], None, (1920.0, 1080.0)), None);
    }
}
