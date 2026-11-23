//! Continuous Keys: one keyboard and mouse for two computers. A hotkey takes the real
//! devices of one computer over and its events appear on the other as virtual ones.
//!
//! This crate is the part that touches the devices. It needs permission to read
//! `/dev/input` on the computer that shares and to write `/dev/uinput` on the one that
//! receives, which is why it is a small program of its own and not part of the service.

pub mod chord;
pub mod event;
pub mod router;
