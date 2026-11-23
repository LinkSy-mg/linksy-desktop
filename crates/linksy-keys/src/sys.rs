//! The few ioctls of evdev and uinput, written out here so the crate needs nothing but libc.

use std::io;
use std::os::fd::RawFd;

pub type Req = libc::c_ulong;

const NONE: u32 = 0;
const WRITE: u32 = 1;
const READ: u32 = 2;

const fn ioc(dir: u32, ty: u8, nr: u8, size: usize) -> Req {
    ((dir << 30) | ((size as u32) << 16) | ((ty as u32) << 8) | nr as u32) as Req
}

pub const EVIOCGRAB: Req = ioc(WRITE, b'E', 0x90, 4);
pub const fn eviocgbit(ev: u8, len: usize) -> Req {
    ioc(READ, b'E', 0x20 + ev, len)
}
pub const fn eviocgname(len: usize) -> Req {
    ioc(READ, b'E', 0x06, len)
}

pub const UI_DEV_CREATE: Req = ioc(NONE, b'U', 1, 0);
pub const UI_DEV_DESTROY: Req = ioc(NONE, b'U', 2, 0);
pub const UI_DEV_SETUP: Req = ioc(WRITE, b'U', 3, 92);
pub const UI_SET_EVBIT: Req = ioc(WRITE, b'U', 100, 4);
pub const UI_SET_KEYBIT: Req = ioc(WRITE, b'U', 101, 4);
pub const UI_SET_RELBIT: Req = ioc(WRITE, b'U', 102, 4);

fn check(rc: libc::c_int) -> io::Result<()> {
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

pub fn ioctl_none(fd: RawFd, req: Req) -> io::Result<()> {
    // SAFETY: a request that takes no argument
    check(unsafe { libc::ioctl(fd, req as _) })
}

pub fn ioctl_int(fd: RawFd, req: Req, arg: libc::c_int) -> io::Result<()> {
    // SAFETY: a request that takes an int by value
    check(unsafe { libc::ioctl(fd, req as _, arg) })
}

pub fn ioctl_read(fd: RawFd, req: Req, buf: &mut [u8]) -> io::Result<()> {
    // SAFETY: the request's size field is the length of `buf`, which the kernel fills
    check(unsafe { libc::ioctl(fd, req as _, buf.as_mut_ptr()) })
}

pub fn ioctl_ptr<T>(fd: RawFd, req: Req, value: &T) -> io::Result<()> {
    // SAFETY: `value` outlives the call and the request reads size_of::<T>() bytes from it
    check(unsafe { libc::ioctl(fd, req as _, value as *const T) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_numbers_are_the_kernels() {
        assert_eq!(EVIOCGRAB, 0x4004_4590);
        assert_eq!(eviocgname(256), 0x8100_4506);
        assert_eq!(eviocgbit(0, 4), 0x8004_4520);
        assert_eq!(UI_DEV_CREATE, 0x5501);
        assert_eq!(UI_DEV_DESTROY, 0x5502);
        assert_eq!(UI_DEV_SETUP, 0x405c_5503);
        assert_eq!(UI_SET_EVBIT, 0x4004_5564);
        assert_eq!(UI_SET_KEYBIT, 0x4004_5565);
        assert_eq!(UI_SET_RELBIT, 0x4004_5566);
    }
}
