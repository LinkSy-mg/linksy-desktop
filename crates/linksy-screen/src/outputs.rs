//! The desktop's outputs as Wayland describes them (xdg-output), for KDE's second display:
//! its stream's size is in pixels, but KDE places pointer positions in the desktop's own
//! coordinates, which are smaller when the display is scaled (125 %: a tap meant for the
//! middle landed lower right, desktop#51). GNOME takes positions in the stream's pixels.

use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::{self, WlRegistry};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_manager_v1::{self, ZxdgOutputManagerV1};
use wayland_protocols::xdg::xdg_output::zv1::client::zxdg_output_v1::{self, ZxdgOutputV1};

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
struct Outputs(Vec<Output>);

/// The size on the desktop of KDE's second display that is `size` pixels big: the newest
/// output KDE made for sharing (`Virtual-virtual-xdp-kde-…`) in that mode. None elsewhere,
/// or when Wayland can't tell.
pub fn kde_virtual_size(size: (f64, f64)) -> Option<(f64, f64)> {
    let conn = Connection::connect_to_env().ok()?;
    let (globals, mut queue) = registry_queue_init::<Outputs>(&conn).ok()?;
    let qh = queue.handle();
    let manager: ZxdgOutputManagerV1 = globals.bind(&qh, 2..=3, ()).ok()?;
    let mut state = Outputs::default();
    for global in globals.contents().clone_list().into_iter().filter(|g| g.interface == "wl_output") {
        let i = state.0.len();
        let output: WlOutput = globals.registry().bind(global.name, global.version.min(2), &qh, i);
        manager.get_xdg_output(&output, &qh, i);
        state.0.push(Output { id: global.name, ..Default::default() });
    }
    queue.roundtrip(&mut state).ok()?;
    pick(&state.0, size)
}

fn pick(outputs: &[Output], (w, h): (f64, f64)) -> Option<(f64, f64)> {
    let mode = (w.round() as i32, h.round() as i32);
    outputs
        .iter()
        .filter(|o| o.name.as_deref().is_some_and(|n| n.contains("virtual-xdp-kde")) && o.mode == Some(mode))
        .max_by_key(|o| o.id)
        .and_then(|o| o.logical)
        .filter(|&(w, h)| w > 0 && h > 0)
        .map(|(w, h)| (w as f64, h as f64))
}

impl Dispatch<WlRegistry, GlobalListContents> for Outputs {
    fn event(_: &mut Self, _: &WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZxdgOutputManagerV1, ()> for Outputs {
    fn event(_: &mut Self, _: &ZxdgOutputManagerV1, _: zxdg_output_manager_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlOutput, usize> for Outputs {
    fn event(state: &mut Self, _: &WlOutput, event: wl_output::Event, &i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Mode { flags: WEnum::Value(flags), width, height, .. } = event
            && flags.contains(wl_output::Mode::Current)
        {
            state.0[i].mode = Some((width, height));
        }
    }
}

impl Dispatch<ZxdgOutputV1, usize> for Outputs {
    fn event(state: &mut Self, _: &ZxdgOutputV1, event: zxdg_output_v1::Event, &i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        match event {
            zxdg_output_v1::Event::LogicalSize { width, height } => state.0[i].logical = Some((width, height)),
            zxdg_output_v1::Event::Name { name } => state.0[i].name = Some(name),
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
        assert_eq!(pick(&outputs, (1920.0, 1080.0)), Some((1536.0, 864.0)));
    }

    #[test]
    fn the_newest_of_kdes_displays_in_that_size() {
        let outputs = [
            output(61, "Virtual-virtual-xdp-kde-", (1920, 1080), (1920, 1080)),
            output(75, "Virtual-virtual-xdp-kde--1", (1920, 1080), (1536, 864)),
            output(80, "Virtual-virtual-xdp-kde--2", (2400, 1080), (2400, 1080)),
        ];
        assert_eq!(pick(&outputs, (1920.0, 1080.0)), Some((1536.0, 864.0)));
    }

    #[test]
    fn nothing_but_kdes_own_displays() {
        // GNOME's virtual monitor, or a monitor in that size: the stream's size stays
        let outputs = [output(40, "Meta-0", (1920, 1080), (960, 540)), output(41, "eDP-1", (1920, 1080), (1536, 864))];
        assert_eq!(pick(&outputs, (1920.0, 1080.0)), None);
        assert_eq!(pick(&[], (1920.0, 1080.0)), None);
    }
}
