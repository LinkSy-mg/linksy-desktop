//! Development aid: try the cable's virtual touchscreen and keyboard (Android Open Accessory
//! 2.0) on a plugged-in phone, with no developer options and no Linksy app involved.
//!
//! cargo run -p linksyd --example phone_input [-- --tap X Y] [--type TEXT] [--swipe]
//!
//! X and Y are fractions of the screen (0.5 0.5 is the middle). With no option it taps the
//! middle, then types "hello".

#[path = "../src/usb_hid.rs"]
#[allow(dead_code)]
mod usb_hid;

use std::time::Duration;

use linksy_core::{ControlMsg, MouseButton};
use nusb::transfer::{ControlIn, ControlType, Recipient};

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut found = None;
    for d in nusb::list_devices().await.expect("USB") {
        let Ok(dev) = d.open().await else { continue };
        let asked = dev.control_in(ControlIn { control_type: ControlType::Vendor, recipient: Recipient::Device, request: 51, value: 0, index: 0, length: 2 }, Duration::from_millis(500)).await;
        if let Ok(v) = asked
            && v.len() == 2
        {
            let version = u16::from_le_bytes([v[0], v[1]]);
            println!("{} {:04x}:{:04x}: accessory protocol {version}", d.product_string().unwrap_or("?"), d.vendor_id(), d.product_id());
            if version >= 2 {
                found = Some(dev);
                break;
            }
        }
    }
    let Some(dev) = found else { return println!("no phone that takes virtual input (accessory protocol 2) found") };
    let mut hid = match usb_hid::Hid::register(dev).await {
        Ok(h) => h,
        Err(e) => return println!("{e}"),
    };
    println!("touchscreen and keyboard plugged in");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let opt = |name: &str| args.iter().position(|a| a == name);
    let mut steps = Vec::new();
    if let Some(i) = opt("--tap") {
        let x: f64 = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(0.5);
        let y: f64 = args.get(i + 2).and_then(|v| v.parse().ok()).unwrap_or(0.5);
        steps.extend([ControlMsg::Pointer { x, y }, ControlMsg::Button { button: MouseButton::Left, pressed: true }, ControlMsg::Button { button: MouseButton::Left, pressed: false }]);
    }
    if let Some(i) = opt("--type") {
        // One character at a time, so each one can be seen
        steps.extend(args.get(i + 1).cloned().unwrap_or_default().chars().map(|c| ControlMsg::Text { text: c.to_string() }));
    }
    if opt("--swipe").is_some() {
        steps.extend([ControlMsg::Pointer { x: 0.5, y: 0.6 }, ControlMsg::Scroll { dx: 0.0, dy: 3.0, discrete: true }]);
    }
    if args.is_empty() {
        steps.extend([
            ControlMsg::Pointer { x: 0.5, y: 0.5 },
            ControlMsg::Button { button: MouseButton::Left, pressed: true },
            ControlMsg::Button { button: MouseButton::Left, pressed: false },
            ControlMsg::Text { text: "hello".into() },
        ]);
    }
    for step in &steps {
        match hid.apply(step).await {
            Ok(()) => println!("sent {step:?}"),
            Err(e) => println!("failed {step:?}: {e}"),
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    hid.unregister().await;
    println!("unplugged the virtual devices");
}
