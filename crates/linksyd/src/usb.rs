//! Phones over a USB cable (docs/protocol.md §4.11): lower latency than Wi-Fi for the webcam and
//! the microphone, and it works with no network at all.
//!
//! No developer options on the phone: this computer switches a plugged-in phone into
//! Android Open Accessory mode, which gives a plain two-way byte pipe to the Linksy app.
//! Linksy's own QUIC packets ride that pipe, framed, so everything works as on Wi-Fi,
//! encrypted end to end by the same pinned keys. On this side the bridge is a local UDP
//! socket: packets from the phone reach this node from 127.0.0.1, which is how the node
//! knows the connection is the cable (and prefers it to Wi-Fi).
//!
//! Switching a phone drops its file-transfer (MTP) mode until it's unplugged, and most of
//! the time a phone is plugged in only to charge: so plugging one in does nothing by itself
//! (desktop 0.8.7). The cable is used when someone asks (`connect`: the apps' Connect by
//! USB Cable, which also works before the phone is paired), or on its own for the phones
//! set to "use the cable automatically" (`set_auto`, kept in `usb-auto.json`, off by
//! default). The app shows the phones plugged in and how far each got (`status`).

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use futures_util::StreamExt;
use linksy_core::cable::{self, Frame};
use linksy_core::{Event, Node, PluggedPhone};
use linksy_ipc::{UsbPhone, UsbState, UsbStatus};
use nusb::hotplug::HotplugEvent;
use nusb::transfer::{Bulk, ControlIn, ControlOut, ControlType, In, Out, Recipient};
use nusb::{DeviceId, DeviceInfo};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UdpSocket;
use tracing::{debug, info, warn};

/// Google's ids for a device in accessory mode (with or without ADB and audio).
const AOA_VENDOR: u16 = 0x18d1;
const AOA_PRODUCTS: std::ops::RangeInclusive<u16> = 0x2d00..=0x2d05;
/// What the phone app's accessory filter matches, and the version of the framing
/// (`linksy_core::cable`).
const ACCESSORY: [&str; 4] = ["Linksy", "Linksy", "Linksy over USB", cable::ACCESSORY_VERSION];
const URI: &str = "https://linksy.diary.mg";
/// Phone makers whose phones may show no MTP or ADB interface (charging-only mode): Google,
/// Samsung, Xiaomi, OnePlus, Motorola, LG, Huawei, Sony, OPPO/realme, vivo, HTC, ZTE, Lenovo,
/// ASUS, Meizu, HMD/Nokia, Honor, Sharp, Kyocera, TCL/Alcatel, Fairphone, and two more.
const PHONE_VENDORS: &[u16] = &[
    0x18d1, 0x04e8, 0x2717, 0x2a70, 0x22b8, 0x1004, 0x12d1, 0x0fce, 0x22d9, 0x2d95, 0x0bb4, 0x19d2, 0x17ef, 0x0b05, 0x2a45, 0x2e04, 0x339b, 0x04dd, 0x0482, 0x1bbb, 0x2ae5, 0x2b4c, 0x1ebf,
];
/// Largest packet the bridge carries: QUIC's are far smaller.
const MAX_PACKET: usize = 65_535;
const TIMEOUT: Duration = Duration::from_secs(1);
/// Android gives a switched phone 10 s to come back as an accessory, and some wait for the
/// first unlock: one that came back as it was gets asked again, this many times.
const SWITCH_TRIES: u8 = 3;

pub static USB: OnceLock<Usb> = OnceLock::new();

#[derive(Clone)]
pub struct Usb {
    node: Node,
    choices: Arc<Mutex<Choices>>,
    /// What's plugged in now that looks like a phone, and how far it got.
    plugged: Arc<Mutex<HashMap<DeviceId, (DeviceInfo, UsbPhone)>>>,
    /// Phones' names by key, to name one back in accessory mode (it shows Google's ids).
    names: Arc<Mutex<HashMap<String, String>>>,
    /// Phones whose app never answered over the cable (Android gives up on the accessory
    /// after a few seconds): not switched again by themselves, which would loop.
    unanswered: Arc<Mutex<std::collections::HashSet<String>>>,
    watching: Arc<std::sync::atomic::AtomicBool>,
    /// What connected phones were last told is plugged in (`usb_plugged`, §4.11).
    told: Arc<Mutex<Vec<PluggedPhone>>>,
}

struct Choices {
    path: PathBuf,
    /// USB serial (or vendor:product) → switch it as soon as it's plugged in.
    phones: HashMap<String, bool>,
}

impl Choices {
    fn load(path: PathBuf) -> Self {
        let phones = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Choices { path, phones }
    }

    fn set(&mut self, key: String, yes: bool) {
        self.phones.insert(key, yes);
        if let Err(e) = serde_json::to_vec_pretty(&self.phones).map_err(std::io::Error::other).and_then(|b| std::fs::write(&self.path, b)) {
            warn!("could not save {}: {e}", self.path.display());
        }
    }
}

impl Usb {
    /// `LINKSY_USB=always` switches every phone (tests).
    pub fn new(node: Node, data_dir: PathBuf) -> Self {
        Usb {
            node,
            // Before 0.8.7, usb.json kept the answers to the plug-in question; "use the cable
            // automatically" starts off for every phone
            choices: Arc::new(Mutex::new(Choices::load(data_dir.join("usb-auto.json")))),
            plugged: Arc::default(),
            names: Arc::default(),
            unanswered: Arc::default(),
            watching: Arc::default(),
            told: Arc::default(),
        }
    }

    pub async fn run(self) {
        let mut watch = match nusb::watch_devices() {
            Ok(w) => w,
            Err(e) => return debug!("no USB hotplug: {e}"),
        };
        self.watching.store(true, std::sync::atomic::Ordering::Relaxed);
        tokio::spawn(self.clone().answer_phones());
        if let Ok(list) = nusb::list_devices().await {
            for d in list {
                self.clone().plugged(d);
            }
        }
        while let Some(event) = watch.next().await {
            match event {
                HotplugEvent::Connected(d) => self.clone().plugged(d),
                HotplugEvent::Disconnected(id) => {
                    self.plugged.lock().unwrap().remove(&id);
                    self.tell_phones();
                }
            }
        }
    }

    pub fn status(&self) -> UsbStatus {
        let mut phones: Vec<UsbPhone> = self.plugged.lock().unwrap().values().map(|(_, p)| p.clone()).collect();
        phones.sort_by(|a, b| a.name.cmp(&b.name));
        UsbStatus { watching: self.watching.load(std::sync::atomic::Ordering::Relaxed), phones }
    }

    /// Use a plugged-in phone with Linksy now (the apps' Connect by USB Cable).
    pub async fn connect(&self, key: &str) -> Result<(), String> {
        let found = self.plugged.lock().unwrap().iter().find(|(_, (_, p))| p.key == key).map(|(id, (d, _))| (*id, d.clone()));
        let (id, d) = found.ok_or("that phone isn't plugged in anymore")?;
        self.unanswered.lock().unwrap().remove(key);
        self.switch(id, &d).await
    }

    /// Switch this phone on its own whenever it's plugged in, or stop.
    pub fn set_auto(&self, key: &str, on: bool) {
        self.choices.lock().unwrap().set(key.to_string(), on);
        for (_, p) in self.plugged.lock().unwrap().values_mut() {
            if p.key == key {
                p.auto = on;
            }
        }
    }

    fn set_state(&self, id: DeviceId, state: UsbState, error: Option<String>) {
        if let Some((_, p)) = self.plugged.lock().unwrap().get_mut(&id) {
            p.state = state;
            p.error = error;
        }
        self.tell_phones();
    }

    /// The phones plugged in, as `usb_plugged` gives them.
    fn plugged_phones(&self) -> Vec<PluggedPhone> {
        let mut phones: Vec<PluggedPhone> = self
            .plugged
            .lock()
            .unwrap()
            .values()
            .filter(|(_, p)| p.state != UsbState::Declined)
            .map(|(d, p)| PluggedPhone {
                key: p.key.clone(),
                manufacturer: d.manufacturer_string().unwrap_or_default().to_string(),
                product: p.name.clone(),
                state: serde_json::to_value(p.state).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default(),
            })
            .collect();
        phones.sort_by(|a, b| a.key.cmp(&b.key));
        phones
    }

    /// Paired Android phones connected now.
    fn connected_phones(&self) -> Vec<linksy_core::Fingerprint> {
        self.node.devices().into_iter().filter(|d| d.trusted && d.connected && d.device.os == "android").map(|d| d.device.id).collect()
    }

    /// Tell connected phones what's plugged in, when that changed.
    fn tell_phones(&self) {
        let phones = self.plugged_phones();
        {
            let mut told = self.told.lock().unwrap();
            if *told == phones {
                return;
            }
            *told = phones.clone();
        }
        for id in self.connected_phones() {
            let (node, phones) = (self.node.clone(), phones.clone());
            tokio::spawn(async move {
                let _ = node.usb_plugged(&id, phones).await;
            });
        }
    }

    /// A phone that connects hears what's plugged in; a phone's own Connect by USB Cable
    /// switches it, as the computer's button does.
    async fn answer_phones(self) {
        let mut events = self.node.subscribe();
        loop {
            match events.recv().await {
                Ok(Event::Connected { device }) if device.os == "android" => {
                    let phones = self.plugged_phones();
                    let _ = self.node.usb_plugged(&device.id, phones).await;
                }
                Ok(Event::UsbConnect { from, key }) => {
                    info!("{} asked to use the cable", from.name);
                    let usb = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = usb.connect(&key).await {
                            warn!("cable for {}: {e}", from.name);
                        }
                    });
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    }

    async fn switch(&self, id: DeviceId, d: &DeviceInfo) -> Result<(), String> {
        self.set_state(id, UsbState::Switching, None);
        let result = switch(d).await;
        match &result {
            Ok(()) => self.check_switched(key_of(d), 1),
            Err(e) => {
                warn!("could not switch {} to Linksy over USB: {e}", d.product_string().unwrap_or("the phone"));
                self.set_state(id, UsbState::Failed, Some(e.clone()));
            }
        }
        result
    }

    /// A few seconds after a switch: the phone should be an accessory by now. One that came
    /// back as it was (Android's window passed, or it waited for an unlock) is asked again.
    fn check_switched(&self, key: String, tries: u8) {
        let usb = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(8)).await;
            let back = usb.plugged.lock().unwrap().iter().find(|(_, (_, p))| p.key == key).map(|(id, (d, p))| (*id, d.clone(), p.state));
            let Some((id, d, state)) = back else { return };
            // An accessory came: whatever happened next isn't Android's window
            if state == UsbState::Accessory || usb.unanswered.lock().unwrap().contains(&key) {
                return;
            }
            if tries >= SWITCH_TRIES {
                warn!("{} never came back as an accessory", d.product_string().unwrap_or("the phone"));
                usb.set_state(id, UsbState::Failed, Some("The phone didn't switch to Linksy. Unlock it, then try again".into()));
                return;
            }
            info!("switching {} again (try {})", d.product_string().unwrap_or("the phone"), tries + 1);
            match switch(&d).await {
                Ok(()) => usb.check_switched(key, tries + 1),
                Err(e) => usb.set_state(id, UsbState::Failed, Some(e)),
            }
        });
    }

    fn plugged(self, d: DeviceInfo) {
        let key = key_of(&d);
        if d.vendor_id() == AOA_VENDOR && AOA_PRODUCTS.contains(&d.product_id()) {
            // Most phones keep their serial in accessory mode: name it as before.
            let name = self.names.lock().unwrap().get(&key).cloned().unwrap_or_else(|| "Your phone".into());
            let id = d.id();
            self.plugged.lock().unwrap().insert(id, (d.clone(), UsbPhone { key: key.clone(), name, state: UsbState::Accessory, error: None, auto: false }));
            self.tell_phones();
            tokio::spawn(async move {
                match bridge(&d, self.node.local_addr().map(|a| a.port()).unwrap_or(0)).await {
                    Ok(0) => {
                        info!("USB link ended: the phone app never answered");
                        self.unanswered.lock().unwrap().insert(key);
                    }
                    Ok(_) => info!("USB link ended"),
                    Err(e) => {
                        warn!("USB link: {e}");
                        self.set_state(id, UsbState::Failed, Some(e));
                    }
                }
            });
            return;
        }
        if !looks_like_phone(&d) {
            return;
        }
        let name = d.product_string().unwrap_or("A phone").to_string();
        self.names.lock().unwrap().insert(key.clone(), name.clone());
        let auto = self.choices.lock().unwrap().phones.get(&key).copied().unwrap_or(false);
        let unanswered = self.unanswered.lock().unwrap().contains(&key);
        let (state, error) = if unanswered {
            (UsbState::Failed, Some("Linksy on the phone didn't take the cable. Open Linksy on the phone first".to_string()))
        } else {
            (UsbState::Plugged, None)
        };
        let id = d.id();
        self.plugged.lock().unwrap().insert(id, (d.clone(), UsbPhone { key: key.clone(), name, state, error, auto }));
        self.tell_phones();
        // Nothing asked: on its own only for a phone set to use the cable automatically
        // (once an Android phone is paired), and not again after its app didn't answer
        let always = std::env::var("LINKSY_USB").is_ok_and(|v| v == "always");
        if !(auto || always) || !self.has_phone() || unanswered {
            return;
        }
        tokio::spawn(async move {
            let _ = self.switch(id, &d).await;
        });
    }

    /// Only bother when an Android phone is paired.
    fn has_phone(&self) -> bool {
        self.node.devices().iter().any(|d| d.trusted && d.device.os == "android")
    }
}

/// A phone's key: its USB serial, which most phones keep in accessory mode.
fn key_of(d: &DeviceInfo) -> String {
    d.serial_number().map(str::to_string).unwrap_or_else(|| format!("{:04x}:{:04x}", d.vendor_id(), d.product_id()))
}

/// An Android phone: it shows a file-transfer (MTP/PTP) or ADB interface, or it's from a
/// phone maker and isn't a disk, keyboard, camera or sound card.
fn looks_like_phone(d: &DeviceInfo) -> bool {
    let kinds: Vec<(u8, u8, u8)> = d.interfaces().map(|i| (i.class(), i.subclass(), i.protocol())).collect();
    if kinds.iter().any(|k| matches!(k, (0x06, 0x01, 0x01) | (0xff, 0x42, 0x01))) || d.interfaces().any(|i| i.interface_string() == Some("MTP")) {
        return true;
    }
    PHONE_VENDORS.contains(&d.vendor_id()) && !kinds.iter().any(|(class, ..)| matches!(class, 0x01 | 0x03 | 0x08 | 0x0e))
}

/// Ask the phone to restart as a Linksy accessory (Android Open Accessory 1.0): it
/// disconnects and comes back with Google's accessory ids, and the phone offers to open
/// Linksy (or its website, when the app isn't installed).
async fn switch(d: &DeviceInfo) -> Result<(), String> {
    let dev = d.open().await.map_err(denied)?;
    let version = dev
        .control_in(ControlIn { control_type: ControlType::Vendor, recipient: Recipient::Device, request: 51, value: 0, index: 0, length: 2 }, TIMEOUT)
        .await
        .map_err(|e| format!("not an Android phone ({e})"))?;
    if version.len() < 2 || u16::from_le_bytes([version[0], version[1]]) == 0 {
        return Err("doesn't support accessories".into());
    }
    let strings = ACCESSORY.into_iter().chain([URI]);
    for (index, s) in strings.enumerate() {
        let mut data = s.as_bytes().to_vec();
        data.push(0);
        dev.control_out(ControlOut { control_type: ControlType::Vendor, recipient: Recipient::Device, request: 52, value: 0, index: index as u16, data: &data }, TIMEOUT)
            .await
            .map_err(|e| e.to_string())?;
    }
    dev.control_out(ControlOut { control_type: ControlType::Vendor, recipient: Recipient::Device, request: 53, value: 0, index: 0, data: &[] }, TIMEOUT)
        .await
        .map_err(|e| e.to_string())?;
    info!("switched {} to Linksy over USB", d.product_string().unwrap_or("a phone"));
    Ok(())
}

fn denied(e: nusb::Error) -> String {
    if e.kind() == nusb::ErrorKind::PermissionDenied {
        "no permission to use USB phones: Finish Setup in Linksy adds it".into()
    } else {
        e.to_string()
    }
}

/// Carry the phone's packets to this node and back until the cable is pulled, framed as
/// `linksy_core::cable` says. Ends with how many packets came from the phone (none: its app
/// never opened the cable).
async fn bridge(d: &DeviceInfo, port: u16) -> Result<u64, String> {
    let dev = d.open().await.map_err(denied)?;
    let intf = dev.detach_and_claim_interface(0).await.map_err(|e| e.to_string())?;
    let desc = intf.descriptor().ok_or("no accessory interface")?;
    let bulk = |dir_in: bool| desc.endpoints().find(|e| e.transfer_type() == nusb::descriptors::TransferType::Bulk && (e.address() & 0x80 != 0) == dir_in).map(|e| e.address());
    let (ep_in, ep_out) = (bulk(true).ok_or("no bulk IN endpoint")?, bulk(false).ok_or("no bulk OUT endpoint")?);
    // Endpoints as they are: clearing a halt here breaks bulk transfers on some Samsungs
    let usb_rx = intf.endpoint::<Bulk, In>(ep_in).map_err(|e| e.to_string())?.reader(cable::READ).with_num_transfers(4);
    let usb_tx = intf.endpoint::<Bulk, Out>(ep_out).map_err(|e| e.to_string())?.writer(cable::READ).with_num_transfers(4);

    let udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.map_err(|e| e.to_string())?;
    udp.connect(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).await.map_err(|e| e.to_string())?;
    info!("Linksy over USB: {}", d.product_string().unwrap_or("a phone"));
    Ok(carry(usb_rx, usb_tx, &udp).await)
}

/// The cable towards the phone: writes are buffered into USB transfers, and each batch ends
/// on a short or empty packet so the phone's read returns at once.
trait CableOut: AsyncWrite + Unpin {
    fn end(&mut self);
}

impl CableOut for nusb::io::EndpointWrite<Bulk> {
    fn end(&mut self) {
        self.submit_end();
    }
}

/// Move packets between the cable and `udp` (connected to this node) until either side
/// stops, or a phone that speaks version 2 goes quiet.
async fn carry(mut usb_rx: impl AsyncRead + Unpin, mut usb_tx: impl CableOut, udp: &UdpSocket) -> u64 {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
    // How much crossed each way, for the log: a cable where nothing comes back from the
    // phone means the phone app never opened its side.
    let (from_phone, to_phone) = (AtomicU64::new(0), AtomicU64::new(0));
    let v2 = AtomicBool::new(false);
    let heard = Mutex::new(std::time::Instant::now());
    let (answer, mut answers) = tokio::sync::mpsc::unbounded_channel::<Frame>();
    let phone_to_node = async {
        let mut buf = vec![0u8; cable::READ];
        let mut frames = cable::Decoder::default();
        loop {
            let n = usb_rx.read(&mut buf).await?;
            if n == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            *heard.lock().unwrap() = std::time::Instant::now();
            frames.push(&buf[..n]);
            while let Some(frame) = frames.next_frame() {
                match frame {
                    Frame::Data(packet) => {
                        if from_phone.fetch_add(1, Relaxed) == 0 {
                            info!("USB link: first packet from the phone");
                        }
                        // The node may be busy; a lost packet is QUIC's to resend.
                        let _ = udp.send(&packet).await;
                    }
                    Frame::Hello { version } => {
                        info!("USB link: the phone speaks version {version}");
                        v2.store(true, Relaxed);
                    }
                    Frame::Ping(t) => {
                        let _ = answer.send(Frame::Pong(t));
                    }
                    Frame::Close => return Ok(()),
                    _ => {}
                }
            }
        }
    };
    let node_to_phone = async {
        let mut buf = vec![0u8; MAX_PACKET];
        let mut tick = tokio::time::interval(cable::PING_EVERY);
        let started = std::time::Instant::now();
        loop {
            let mut bytes = Vec::with_capacity(cable::READ);
            tokio::select! {
                got = udp.recv(&mut buf) => {
                    let len = got?;
                    if to_phone.fetch_add(1, Relaxed) == 0 {
                        info!("USB link: first packet to the phone");
                    }
                    cable::encode(&Frame::Data(buf[..len].to_vec()), &mut bytes);
                    // What's already waiting goes in the same batch
                    while bytes.len() < cable::READ && let Ok(len) = udp.try_recv(&mut buf) {
                        cable::encode(&Frame::Data(buf[..len].to_vec()), &mut bytes);
                    }
                }
                Some(frame) = answers.recv() => cable::encode(&frame, &mut bytes),
                _ = tick.tick() => {
                    if !v2.load(Relaxed) {
                        continue;
                    }
                    if heard.lock().unwrap().elapsed() > cable::SILENT_FOR {
                        warn!("USB link: the phone went quiet");
                        return Err(std::io::ErrorKind::TimedOut.into());
                    }
                    cable::encode(&Frame::Ping(started.elapsed().as_millis() as u64), &mut bytes);
                }
            }
            usb_tx.write_all(&bytes).await?;
            usb_tx.end();
        }
    };
    let ended: std::io::Result<()> = tokio::select! {
        r = phone_to_node => r,
        r = node_to_phone => r,
    };
    let from_phone = from_phone.into_inner();
    info!("USB link: {from_phone} packets from the phone, {} to it", to_phone.into_inner());
    // Unplugged (or the app let go of the accessory) is the normal end.
    if let Err(e) = ended
        && !matches!(e.kind(), std::io::ErrorKind::UnexpectedEof | std::io::ErrorKind::BrokenPipe | std::io::ErrorKind::ConnectionReset)
    {
        debug!("USB link: {e}");
    }
    from_phone
}

#[cfg(test)]
mod tests {
    use super::*;
    use linksy_core::{Config, Event};

    impl CableOut for tokio::io::WriteHalf<tokio::io::DuplexStream> {
        fn end(&mut self) {}
    }

    impl CableOut for tokio::net::unix::OwnedWriteHalf {
        fn end(&mut self) {}
    }

    /// This computer's address on its network, if it has one (the "Wi-Fi" path).
    fn lan_ip() -> Option<std::net::IpAddr> {
        let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
        s.connect("192.0.2.1:9").ok()?;
        let ip = s.local_addr().ok()?.ip();
        (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
    }

    async fn node(dir: &std::path::Path, name: &str) -> Node {
        let mut c = Config::new(dir.join(name), dir.join(format!("{name}-dl")), name);
        c.discovery = false;
        Node::start(c).await.unwrap()
    }

    fn answer_yes(node: &Node) {
        let (node, mut rx) = (node.clone(), node.subscribe());
        tokio::spawn(async move {
            while let Ok(ev) = rx.recv().await {
                if let Event::PairingRequest { request, .. } = ev {
                    node.respond(request, true);
                }
            }
        });
    }

    /// A phone paired over the network plugs in: its QUIC runs through the framed cable,
    /// the computer marks it as USB and uses it instead of the network connection.
    #[tokio::test]
    async fn quic_rides_the_cable_and_wins_over_the_network() {
        let Some(ip) = lan_ip() else { return eprintln!("no network address: skipped") };
        let dir = tempfile::tempdir().unwrap();
        let (pc, phone) = (node(dir.path(), "pc").await, node(dir.path(), "phone").await);
        answer_yes(&pc);
        answer_yes(&phone);
        let port = pc.local_addr().unwrap().port();
        phone.pair(SocketAddr::new(ip, port)).await.unwrap();
        let id = phone.info().id;
        let usb_of = |pc: &Node| pc.devices().into_iter().find(|d| d.device.id == id).map(|d| (d.connected, d.usb));
        let until = |want| {
            let pc = pc.clone();
            async move {
                tokio::time::timeout(Duration::from_secs(5), async {
                    while usb_of(&pc) != want {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                })
                .await
                .is_ok()
            }
        };
        assert!(until(Some((true, false))).await, "pairing over the network");

        // The cable: an in-memory pipe, the computer's bridge at one end, and at the other
        // what the phone app does: a local UDP port that its node dials.
        let (pc_end, phone_end) = tokio::io::duplex(1 << 16);
        let pc_udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        pc_udp.connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
        tokio::spawn(async move {
            let (rx, tx) = tokio::io::split(pc_end);
            carry(rx, tx, &pc_udp).await;
        });
        let phone_udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let phone_port = phone_udp.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut rx, mut tx) = tokio::io::split(phone_end);
            let mut buf = vec![0u8; MAX_PACKET];
            // The phone node's address is learned from its first packet.
            let (len, from) = phone_udp.recv_from(&mut buf).await.unwrap();
            phone_udp.connect(from).await.unwrap();
            tx.write_u16(len as u16).await.unwrap();
            tx.write_all(&buf[..len]).await.unwrap();
            let up = async {
                let mut buf = vec![0u8; MAX_PACKET];
                while let Ok(len) = phone_udp.recv(&mut buf).await {
                    tx.write_u16(len as u16).await?;
                    tx.write_all(&buf[..len]).await?;
                }
                std::io::Result::Ok(())
            };
            let down = async {
                let mut buf = vec![0u8; MAX_PACKET];
                loop {
                    let len = rx.read_u16().await? as usize;
                    rx.read_exact(&mut buf[..len]).await?;
                    phone_udp.send(&buf[..len]).await?;
                }
                #[allow(unreachable_code)]
                std::io::Result::Ok(())
            };
            let _ = tokio::join!(up, down);
        });

        let mut events = phone.subscribe();
        phone.connect(SocketAddr::from((Ipv4Addr::LOCALHOST, phone_port))).await.unwrap();
        assert!(until(Some((true, true))).await, "the computer never saw the cable");

        // And it carries traffic.
        let pc_id = pc.info().id;
        pc.send_clipboard(&phone.info().id, "over the cable").await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(Event::Clipboard { from, text }) = events.recv().await
                    && from.id == pc_id
                {
                    return text;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(got, "over the cable");
    }

    /// The phone app's own bridge (`linksy_core::cable::phone_bridge`, version 2) over a real
    /// two-way pipe: the hello, keepalives and QUIC all cross, and the computer sees the cable.
    #[tokio::test]
    async fn version_two_phone_bridge() {
        let Some(ip) = lan_ip() else { return eprintln!("no network address: skipped") };
        let dir = tempfile::tempdir().unwrap();
        let (pc, phone) = (node(dir.path(), "pc").await, node(dir.path(), "phone").await);
        answer_yes(&pc);
        answer_yes(&phone);
        let port = pc.local_addr().unwrap().port();
        phone.pair(SocketAddr::new(ip, port)).await.unwrap();

        let (pc_end, phone_end) = std::os::unix::net::UnixStream::pair().unwrap();
        pc_end.set_nonblocking(true).unwrap();
        let (rx, tx) = tokio::net::UnixStream::from_std(pc_end).unwrap().into_split();
        let pc_udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        pc_udp.connect((Ipv4Addr::LOCALHOST, port)).await.unwrap();
        tokio::spawn(async move { carry(rx, tx, &pc_udp).await });
        let accessory = std::fs::File::from(std::os::fd::OwnedFd::from(phone_end));
        let bridge_port = cable::phone_bridge(accessory, cable::ACCESSORY_VERSION).unwrap();

        let mut events = phone.subscribe();
        phone.connect(SocketAddr::from((Ipv4Addr::LOCALHOST, bridge_port))).await.unwrap();
        let id = phone.info().id;
        let on_cable = tokio::time::timeout(Duration::from_secs(5), async {
            while !pc.devices().iter().any(|d| d.device.id == id && d.connected && d.usb) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        });
        assert!(on_cable.await.is_ok(), "the computer never saw the cable");
        // Past a few keepalives: still up, still carrying
        tokio::time::sleep(Duration::from_secs(3)).await;
        let pc_id = pc.info().id;
        pc.send_clipboard(&id, "version two").await.unwrap();
        let got = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(Event::Clipboard { from, text }) = events.recv().await
                    && from.id == pc_id
                {
                    return text;
                }
            }
        });
        assert_eq!(got.await.unwrap(), "version two");
    }

    /// A phone that said hello in version 2 and then goes silent is let go within seconds.
    #[tokio::test]
    async fn a_silent_phone_is_let_go() {
        let (pc_end, mut phone_end) = tokio::io::duplex(1 << 16);
        let pc_udp = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        pc_udp.connect((Ipv4Addr::LOCALHOST, 9)).await.unwrap();
        let mut hello = Vec::new();
        cable::encode(&Frame::Hello { version: cable::VERSION }, &mut hello);
        phone_end.write_all(&hello).await.unwrap();
        let (rx, tx) = tokio::io::split(pc_end);
        let started = std::time::Instant::now();
        // Read and drop the computer's pings, never answering
        tokio::spawn(async move {
            let mut buf = vec![0u8; 4096];
            while phone_end.read(&mut buf).await.is_ok_and(|n| n > 0) {}
        });
        let ended = tokio::time::timeout(Duration::from_secs(10), carry(rx, tx, &pc_udp)).await;
        assert!(ended.is_ok(), "still waiting after 10 s");
        assert!(started.elapsed() >= cable::SILENT_FOR, "let go too early");
    }
}
