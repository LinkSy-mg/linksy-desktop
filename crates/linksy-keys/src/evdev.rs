//! The real keyboards and mice of this computer: found, read and, while Continuous Keys is
//! on the other computer, grabbed so that nothing here sees them.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use crate::event::{BTN_LEFT, EV_KEY, EV_REL, Event, KEY_A, KEY_ENTER, REL_X, REL_Y};
use crate::sys;

/// What a virtual device made by this crate is called, so it is never taken for a real one.
pub const VIRTUAL_NAME: &str = "Linksy Keys";

const EV_ABS: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Keyboard,
    Mouse,
}

pub struct Device {
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
    file: File,
}

#[repr(C)]
struct RawEvent {
    time: libc::timeval,
    kind: u16,
    code: u16,
    value: i32,
}

fn bit(bits: &[u8], n: usize) -> bool {
    bits.get(n / 8).is_some_and(|b| b & (1 << (n % 8)) != 0)
}

impl Device {
    fn probe(path: PathBuf) -> io::Result<Option<Self>> {
        let file = OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open(&path)?;
        let fd = file.as_raw_fd();
        let mut name = [0u8; 256];
        sys::ioctl_read(fd, sys::eviocgname(name.len()), &mut name)?;
        let name = String::from_utf8_lossy(&name[..name.iter().position(|&b| b == 0).unwrap_or(0)]).into_owned();
        if name.starts_with(VIRTUAL_NAME) {
            return Ok(None);
        }
        let mut types = [0u8; 4];
        sys::ioctl_read(fd, sys::eviocgbit(0, types.len()), &mut types)?;
        // Touchpads, touchscreens and tablets report absolute positions: they stay here
        if bit(&types, EV_ABS) {
            return Ok(None);
        }
        let mut keys = [0u8; 96];
        if bit(&types, EV_KEY as usize) {
            sys::ioctl_read(fd, sys::eviocgbit(EV_KEY as u8, keys.len()), &mut keys)?;
        }
        let mut rel = [0u8; 2];
        if bit(&types, EV_REL as usize) {
            sys::ioctl_read(fd, sys::eviocgbit(EV_REL as u8, rel.len()), &mut rel)?;
        }
        let kind = if bit(&rel, REL_X as usize) && bit(&rel, REL_Y as usize) && bit(&keys, BTN_LEFT as usize) {
            Kind::Mouse
        } else if bit(&keys, KEY_A as usize) && bit(&keys, KEY_ENTER as usize) {
            Kind::Keyboard
        } else {
            return Ok(None);
        };
        Ok(Some(Self { path, name, kind, file }))
    }

    fn grab(&self, on: bool) -> io::Result<()> {
        sys::ioctl_int(self.file.as_raw_fd(), sys::EVIOCGRAB, on as libc::c_int)
    }
}

/// Every keyboard and mouse that can be shared. The first error is the one to show: without
/// permission to read the devices there is nothing to share.
pub fn list() -> io::Result<Vec<Device>> {
    let mut found = Vec::new();
    let mut first_error = None;
    for entry in fs::read_dir("/dev/input")?.flatten() {
        let path = entry.path();
        if !path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("event")) {
            continue;
        }
        match Device::probe(path) {
            Ok(Some(device)) => found.push(device),
            Ok(None) => {}
            Err(e) => {
                first_error.get_or_insert(e);
            }
        }
    }
    match (found.is_empty(), first_error) {
        (true, Some(e)) => Err(e),
        _ => Ok(found),
    }
}

pub struct Capture {
    devices: Vec<Device>,
}

impl Capture {
    pub fn new(devices: Vec<Device>) -> Self {
        Self { devices }
    }

    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    pub fn grab(&self, on: bool) -> io::Result<()> {
        for device in &self.devices {
            device.grab(on)?;
        }
        Ok(())
    }

    /// What arrived within `timeout_ms`, from every device. A device that was unplugged is
    /// dropped from the list.
    pub fn poll(&mut self, timeout_ms: i32) -> io::Result<Vec<Event>> {
        let mut fds: Vec<libc::pollfd> = self.devices.iter().map(|d| libc::pollfd { fd: d.file.as_raw_fd(), events: libc::POLLIN, revents: 0 }).collect();
        // SAFETY: `fds` is a valid array of `fds.len()` pollfd
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if rc < 0 {
            let e = io::Error::last_os_error();
            return if e.kind() == io::ErrorKind::Interrupted { Ok(Vec::new()) } else { Err(e) };
        }
        let mut out = Vec::new();
        let mut gone = Vec::new();
        for (i, fd) in fds.iter().enumerate() {
            if fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                gone.push(i);
            } else if fd.revents & libc::POLLIN != 0 {
                match read_events(&mut self.devices[i].file, &mut out) {
                    Ok(()) => {}
                    Err(e) if e.raw_os_error() == Some(libc::ENODEV) => gone.push(i),
                    Err(e) => return Err(e),
                }
            }
        }
        for i in gone.into_iter().rev() {
            self.devices.remove(i);
        }
        Ok(out)
    }
}

fn read_events(file: &mut File, out: &mut Vec<Event>) -> io::Result<()> {
    const SIZE: usize = std::mem::size_of::<RawEvent>();
    let mut buf = [0u8; SIZE * 64];
    loop {
        match file.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                let mut at = 0;
                while at + SIZE <= n {
                    // SAFETY: SIZE bytes are in range, and RawEvent is a plain-data struct
                    let raw = unsafe { std::ptr::read_unaligned(buf[at..].as_ptr() as *const RawEvent) };
                    let event = Event { kind: raw.kind, code: raw.code, value: raw.value };
                    if event.carried() {
                        out.push(event);
                    }
                    at += SIZE;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

/// One event as the kernel wants it written, for a virtual device.
pub fn raw_bytes(event: Event) -> [u8; std::mem::size_of::<RawEvent>()] {
    let raw = RawEvent { time: libc::timeval { tv_sec: 0, tv_usec: 0 }, kind: event.kind, code: event.code, value: event.value };
    // SAFETY: RawEvent is plain data; its padding, if any, is zero from the initialiser above
    unsafe { std::mem::transmute::<RawEvent, [u8; std::mem::size_of::<RawEvent>()]>(raw) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_raw_event_is_what_the_kernel_expects() {
        assert_eq!(std::mem::size_of::<RawEvent>(), if cfg!(target_pointer_width = "64") { 24 } else { 16 });
        let bytes = raw_bytes(Event::key(30, true));
        let size = bytes.len();
        assert_eq!(&bytes[size - 8..size - 4], &[EV_KEY as u8, 0, 30, 0]);
        assert_eq!(&bytes[size - 4..], &1i32.to_le_bytes());
    }

    #[test]
    fn bits_are_read_the_way_the_kernel_packs_them() {
        let bits = [0b0000_0100, 0b1000_0000];
        assert!(bit(&bits, 2) && bit(&bits, 15));
        assert!(!bit(&bits, 3) && !bit(&bits, 99));
    }
}
