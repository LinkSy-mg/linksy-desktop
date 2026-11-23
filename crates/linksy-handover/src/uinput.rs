//! The keyboard and mouse this computer is given while it is the other one's: a virtual
//! device that exists only for as long as the sharing lasts. It sends key positions, so
//! this computer's own keyboard layout applies, as if the keyboard had been plugged in here.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;

use crate::event::{EV_KEY, EV_REL, Event, KEY_MAX, REL_HWHEEL, REL_HWHEEL_HI_RES, REL_WHEEL, REL_WHEEL_HI_RES, REL_X, REL_Y};
use crate::evdev::{VIRTUAL_NAME, raw_bytes};
use crate::sys;

#[repr(C)]
struct UinputSetup {
    bustype: u16,
    vendor: u16,
    product: u16,
    version: u16,
    name: [u8; 80],
    ff_effects_max: u32,
}

const BUS_VIRTUAL: u16 = 0x06;

pub struct VirtualDevice {
    file: File,
}

impl VirtualDevice {
    pub fn create() -> io::Result<Self> {
        let file = OpenOptions::new().write(true).custom_flags(libc::O_NONBLOCK | libc::O_CLOEXEC).open("/dev/uinput")?;
        let fd = file.as_raw_fd();
        sys::ioctl_int(fd, sys::UI_SET_EVBIT, EV_KEY as libc::c_int)?;
        for code in 1..=KEY_MAX {
            sys::ioctl_int(fd, sys::UI_SET_KEYBIT, code as libc::c_int)?;
        }
        sys::ioctl_int(fd, sys::UI_SET_EVBIT, EV_REL as libc::c_int)?;
        for code in [REL_X, REL_Y, REL_HWHEEL, REL_WHEEL, REL_WHEEL_HI_RES, REL_HWHEEL_HI_RES] {
            sys::ioctl_int(fd, sys::UI_SET_RELBIT, code as libc::c_int)?;
        }
        let mut setup = UinputSetup { bustype: BUS_VIRTUAL, vendor: 0x1d6b, product: 0x4c4b, version: 1, name: [0; 80], ff_effects_max: 0 };
        setup.name[..VIRTUAL_NAME.len()].copy_from_slice(VIRTUAL_NAME.as_bytes());
        sys::ioctl_ptr(fd, sys::UI_DEV_SETUP, &setup)?;
        sys::ioctl_none(fd, sys::UI_DEV_CREATE)?;
        Ok(Self { file })
    }

    pub fn emit(&mut self, event: Event) -> io::Result<()> {
        self.file.write_all(&raw_bytes(event))
    }
}

impl Drop for VirtualDevice {
    fn drop(&mut self) {
        let _ = sys::ioctl_none(self.file.as_raw_fd(), sys::UI_DEV_DESTROY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_setup_struct_is_what_the_kernel_reads() {
        assert_eq!(std::mem::size_of::<UinputSetup>(), 92);
        assert!(VIRTUAL_NAME.len() < 80);
    }
}
