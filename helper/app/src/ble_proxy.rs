//! The proxy: what Home Assistant sees of this TV, speaking the ESPHome native API the way LumaShow's `ble-proxy` plugin does.
//!
//! No I/O in here: the runner (`ble`) moves bytes between sockets and this, hands it the advertisements LG's service reported
//! and tells it what the radio is doing. The protocol is `esphome-api`'s (the same crate the Show's plugin is built on: Noise
//! `NNpsk0` framing, the messages, the handshake), so Home Assistant's ESPHome integration adds the TV as it adds an ESPHome
//! Bluetooth proxy (docs/research/ble-result.md has the reasoning and the steps).
//!
//! What it is, and is not:
//!
//! - **A passive scanner only.** The features it declares are passive scanning, raw advertisements and scanner state and mode.
//!   Not active connections, not pairing, not the cache variants: LG's service keeps the controller, and a connection to a
//!   sensor is not something to hand a stack that also carries the Magic Remote. A request to connect is answered "not
//!   connected".
//! - **One subscriber.** The connection that subscribes to advertisements gets them, raw, in batches of at most 16 every 100 ms
//!   or sooner. The newest subscriber wins. With none, [`Proxy::wanted`] is false and the runner leaves the radio alone.
//! - **Scanner state is the runner's to tell**: `Running` while a subscriber is there and the duty machine is working (the rests
//!   between windows are not a stopped scanner), `Failed` when it has given up.
//! - **The key is the owner's**: a request to change it is answered "no".

use esphome_api::message::{self as m, Message, feature, scanner};
use esphome_api::session::Server;
use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

/// The ESPHome release whose native API this speaks (what `esphome_version` says: Home Assistant compares it with the oldest
/// release that has the Bluetooth proxy behaviour it relies on). The same as the Show's plugin says.
pub const ESPHOME_VERSION: &str = "2026.9.0";
pub const MAX_CLIENTS: usize = 4;
/// Advertisements in one message, and how long the first waits for company.
pub const ADVERTISEMENT_BATCH: usize = 16;
pub const ADVERTISEMENT_WAIT: Duration = Duration::from_millis(100);
/// What may wait unwritten for the subscriber before advertisements are dropped, not queued.
pub const MAX_BACKLOG: usize = 256 * 1024;

/// Passive scanning, raw advertisements, scanner state and mode.
pub const FEATURES: u32 =
    feature::PASSIVE_SCAN | feature::RAW_ADVERTISEMENTS | feature::STATE_AND_MODE;

pub type ConnId = u64;

/// Where the handshakes' ephemeral keys come from: the system's, or a fixed sequence in a test.
pub type Random = Box<dyn FnMut(&mut [u8]) + Send>;

/// Who this device is, to Home Assistant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub friendly_name: String,
    /// The Wi-Fi interface's address (`AA:BB:CC:DD:EE:FF`): Home Assistant's identity for the device.
    pub mac: String,
    /// The controller's address (empty if not read yet).
    pub bluetooth_mac: String,
}

/// What the runner is told to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    /// End this connection once what is queued for it is written.
    CloseClient(ConnId),
}

/// What the scanner is doing, as Home Assistant is told it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScanState {
    Idle,
    Starting,
    Running,
    Failed,
}

fn api_state(state: ScanState) -> u32 {
    match state {
        ScanState::Idle => scanner::IDLE,
        ScanState::Starting => scanner::STARTING,
        ScanState::Running => scanner::RUNNING,
        ScanState::Failed => scanner::FAILED,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Status {
    pub clients: usize,
    pub subscribed: bool,
    pub advertisements: u64,
    pub dropped: u64,
}

struct Client {
    server: Server,
    closing: bool,
    /// Bytes written by `server` and not yet taken.
    out: Vec<u8>,
}

pub struct Proxy {
    key: [u8; 32],
    identity: Identity,
    random: Random,
    clients: BTreeMap<ConnId, Client>,
    next_conn: ConnId,
    subscriber: Option<ConnId>,
    configured_mode: u32,
    state: ScanState,
    told: Option<(u32, u32)>,
    batch: Vec<m::RawAdvertisement>,
    batch_since: Option<Instant>,
    actions: VecDeque<Action>,
    advertisements: u64,
    dropped: u64,
}

impl Proxy {
    pub fn new(key: [u8; 32], identity: Identity, random: Random) -> Self {
        Self {
            key,
            identity,
            random,
            clients: BTreeMap::new(),
            next_conn: 1,
            subscriber: None,
            configured_mode: scanner::PASSIVE,
            state: ScanState::Idle,
            told: None,
            batch: Vec::new(),
            batch_since: None,
            actions: VecDeque::new(),
            advertisements: 0,
            dropped: 0,
        }
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// The identity was completed (the controller's address and the Wi-Fi address are known only once LG's service answered):
    /// clients that connect from now on are told it.
    pub fn set_identity(&mut self, identity: Identity) {
        self.identity = identity;
    }

    /// Replies LG's reader had to drop because the runner was behind.
    pub fn add_dropped(&mut self, count: u64) {
        self.dropped += count;
    }

    pub fn status(&self) -> Status {
        Status {
            clients: self.clients.len(),
            subscribed: self.subscriber.is_some(),
            advertisements: self.advertisements,
            dropped: self.dropped,
        }
    }

    /// Whether Home Assistant is listening for advertisements: the only reason for the radio to be asked for anything.
    pub fn wanted(&self) -> bool {
        self.subscriber.is_some()
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        self.actions.drain(..).collect()
    }

    // ---- clients -------------------------------------------------------------------------------------------------------------

    /// A new TCP connection: `None` when there are already as many as the proxy keeps.
    pub fn add_client(&mut self) -> Option<ConnId> {
        if self.clients.len() >= MAX_CLIENTS {
            return None;
        }
        let mut ephemeral = [0u8; 32];
        (self.random)(&mut ephemeral);
        let id = self.next_conn;
        self.next_conn += 1;
        self.clients.insert(
            id,
            Client {
                server: Server::new(self.key, &self.identity.name, &self.identity.mac, ephemeral),
                closing: false,
                out: Vec::new(),
            },
        );
        Some(id)
    }

    pub fn client_ids(&self) -> Vec<ConnId> {
        self.clients.keys().copied().collect()
    }

    /// What a client sent.
    pub fn client_input(&mut self, id: ConnId, bytes: &[u8], now: Instant) {
        let Some(client) = self.clients.get_mut(&id) else {
            return;
        };
        let result = client.server.input(bytes);
        client.out.extend(client.server.take_output());
        let mut messages = Vec::new();
        while let Some((kind, payload)) = client.server.next_message() {
            messages.push((kind, payload));
        }
        if result.is_err() {
            // The reason (a key that does not match, a plaintext client) was queued by the session; then the connection ends.
            client.closing = true;
            self.actions.push_back(Action::CloseClient(id));
        }
        for (kind, payload) in messages {
            // A message that does not read, or one of a type not known, is not the end of the connection: ESPHome ignores them too.
            if let Ok(Some(message)) = Message::decode(kind, &payload) {
                self.on_message(id, message, now);
            }
        }
    }

    /// The bytes to write to the client now.
    pub fn client_output(&mut self, id: ConnId) -> Vec<u8> {
        self.clients
            .get_mut(&id)
            .map(|c| std::mem::take(&mut c.out))
            .unwrap_or_default()
    }

    /// Whether the connection is to be closed (it asked, or must be) once its output is written.
    pub fn client_closing(&self, id: ConnId) -> bool {
        self.clients.get(&id).is_some_and(|c| c.closing)
    }

    /// The connection ended (the socket closed, or the runner closed it).
    pub fn client_gone(&mut self, id: ConnId) {
        if self.clients.remove(&id).is_none() {
            return;
        }
        if self.subscriber == Some(id) {
            self.subscriber = None;
            self.clear_batch();
            self.told = None;
        }
    }

    fn clear_batch(&mut self) {
        self.batch.clear();
        self.batch_since = None;
    }

    fn send(&mut self, id: ConnId, message: impl Into<Message>) {
        let message: Message = message.into();
        if let Some(c) = self.clients.get_mut(&id)
            && c.server.is_ready()
            && c.server.send(message.id(), &message.encode()).is_ok()
        {
            c.out.extend(c.server.take_output());
        }
    }

    fn refuse(&mut self, id: ConnId) {
        if let Some(c) = self.clients.get_mut(&id) {
            c.closing = true;
        }
        self.actions.push_back(Action::CloseClient(id));
    }

    // ---- the API's messages --------------------------------------------------------------------------------------------------------

    fn device_info(&self) -> m::DeviceInfoResponse {
        m::DeviceInfoResponse {
            name: self.identity.name.clone(),
            mac_address: self.identity.mac.clone(),
            esphome_version: ESPHOME_VERSION.into(),
            model: "LG webOS TV".into(),
            project_name: "lumatv.ble-proxy".into(),
            project_version: env!("CARGO_PKG_VERSION").into(),
            bluetooth_proxy_feature_flags: FEATURES,
            manufacturer: "LumaTV".into(),
            friendly_name: self.identity.friendly_name.clone(),
            bluetooth_mac_address: self.identity.bluetooth_mac.clone(),
            api_encryption_supported: true,
            ..Default::default()
        }
    }

    fn on_message(&mut self, id: ConnId, message: Message, _now: Instant) {
        match message {
            Message::HelloRequest(_) => self.send(
                id,
                m::HelloResponse {
                    api_version_major: m::API_VERSION_MAJOR,
                    api_version_minor: m::API_VERSION_MINOR,
                    server_info: ESPHOME_VERSION.into(),
                    name: self.identity.name.clone(),
                },
            ),
            Message::DisconnectRequest(_) => {
                self.send(id, m::DisconnectResponse::default());
                self.refuse(id);
            }
            Message::PingRequest(_) => self.send(id, m::PingResponse::default()),
            Message::DeviceInfoRequest(_) => {
                let info = self.device_info();
                self.send(id, info);
            }
            Message::DeviceCapabilitiesRequest(_) => self.send(
                id,
                m::DeviceCapabilitiesResponse {
                    bluetooth_feature_flags: FEATURES,
                    bluetooth_mac_address: self.identity.bluetooth_mac.clone(),
                },
            ),
            // No entities, no states, no logs, nothing of Home Assistant's to subscribe to: asked, and nothing to say.
            Message::ListEntitiesRequest(_) => {
                self.send(id, m::ListEntitiesDoneResponse::default());
            }
            // The key is the owner's to change, with `ble-link` on the TV.
            Message::NoiseEncryptionSetKeyRequest(_) => {
                self.send(id, m::NoiseEncryptionSetKeyResponse { success: false });
            }
            Message::SubscribeBluetoothLEAdvertisementsRequest(_) => self.subscribe(id),
            Message::UnsubscribeBluetoothLEAdvertisementsRequest(_) => {
                if self.subscriber == Some(id) {
                    self.subscriber = None;
                    self.clear_batch();
                    self.told = None;
                }
            }
            Message::BluetoothScannerSetModeRequest(request) => {
                self.configured_mode = if request.mode == scanner::ACTIVE {
                    scanner::ACTIVE
                } else {
                    scanner::PASSIVE
                };
                self.told = None;
                self.tell_scanner_state();
            }
            // No connections are offered: none free, none allowed, and a request for one is "not connected".
            Message::SubscribeBluetoothConnectionsFreeRequest(_) => {
                self.send(id, m::BluetoothConnectionsFreeResponse::default());
            }
            Message::BluetoothDeviceRequest(request) => {
                self.send(
                    id,
                    m::BluetoothDeviceConnectionResponse {
                        address: request.address,
                        connected: false,
                        mtu: 0,
                        error: 0,
                    },
                );
            }
            _ => {}
        }
    }

    fn subscribe(&mut self, id: ConnId) {
        // The newest subscriber wins: an older one that has not noticed it is gone would otherwise starve the new one.
        self.subscriber = Some(id);
        self.clear_batch();
        self.told = None;
        self.tell_scanner_state();
    }

    // ---- the scanner ---------------------------------------------------------------------------------------------------------------

    fn scanner_state(&self) -> m::BluetoothScannerStateResponse {
        m::BluetoothScannerStateResponse {
            state: api_state(self.state),
            // Passive is all LG's service gives (it hands over advertisements; it sends no scan requests of its own that
            // Home Assistant could ask for): the mode Home Assistant asked for is `configured_mode`.
            mode: scanner::PASSIVE,
            configured_mode: self.configured_mode,
        }
    }

    /// Tells the subscriber the scanner's state when it is not what it was last told.
    fn tell_scanner_state(&mut self) {
        let Some(id) = self.subscriber else {
            return;
        };
        let state = self.scanner_state();
        if self.told != Some((state.state, state.mode)) {
            self.told = Some((state.state, state.mode));
            self.send(id, state);
        }
    }

    /// What the runner says the scanner is doing (it changes with the duty machine and with who is subscribed).
    pub fn set_scan_state(&mut self, state: ScanState) {
        if self.state != state {
            self.state = state;
            self.tell_scanner_state();
        }
    }

    pub fn scan_state(&self) -> ScanState {
        self.state
    }

    /// An advertisement LG's service reported. Dropped when nobody is subscribed.
    pub fn heard(&mut self, advertisement: m::RawAdvertisement, now: Instant) {
        let Some(id) = self.subscriber else {
            return;
        };
        if self
            .clients
            .get(&id)
            .is_none_or(|c| c.out.len() > MAX_BACKLOG)
        {
            self.dropped += 1;
            return;
        }
        self.advertisements += 1;
        self.batch.push(advertisement);
        if self.batch.len() >= ADVERTISEMENT_BATCH {
            self.flush();
        } else {
            self.batch_since.get_or_insert(now);
        }
    }

    fn flush(&mut self) {
        self.batch_since = None;
        if self.batch.is_empty() {
            return;
        }
        let advertisements = std::mem::take(&mut self.batch);
        if let Some(id) = self.subscriber {
            self.send(
                id,
                m::BluetoothLERawAdvertisementsResponse { advertisements },
            );
        }
    }

    /// When the first advertisement of a batch has waited long enough.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.batch_since.map(|t| t + ADVERTISEMENT_WAIT)
    }

    pub fn handle_timeout(&mut self, now: Instant) {
        if self
            .batch_since
            .is_some_and(|t| now >= t + ADVERTISEMENT_WAIT)
        {
            self.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use esphome_api::session::Client as Ha;

    const KEY: [u8; 32] = [0x5a; 32];

    fn identity() -> Identity {
        Identity {
            name: "lumatv-ble".into(),
            friendly_name: "LumaTV Bluetooth".into(),
            mac: "AA:BB:CC:00:11:22".into(),
            bluetooth_mac: "84:A3:29:B9:4D:43".into(),
        }
    }

    fn advert(address: u64) -> m::RawAdvertisement {
        m::RawAdvertisement {
            address,
            rssi: -70,
            address_type: 0,
            data: vec![2, 1, 6],
        }
    }

    struct Rig {
        proxy: Proxy,
        has: Vec<(ConnId, Ha, Vec<Message>)>,
        now: Instant,
        counter: u8,
    }

    impl Rig {
        fn new() -> Self {
            Self {
                proxy: Proxy::new(KEY, identity(), Box::new(|buf: &mut [u8]| buf.fill(9))),
                has: Vec::new(),
                now: Instant::now(),
                counter: 100,
            }
        }

        /// Home Assistant connects, says hello and gets the answer.
        fn connect(&mut self) -> usize {
            let id = self.proxy.add_client().expect("room for a client");
            self.counter += 1;
            self.has
                .push((id, Ha::new(KEY, [self.counter; 32]), Vec::new()));
            self.settle();
            let ha = self.has.len() - 1;
            self.say(
                ha,
                m::HelloRequest {
                    client_info: "Home Assistant 2026.9.4".into(),
                    api_version_major: 1,
                    api_version_minor: 19,
                },
            );
            self.take(ha);
            ha
        }

        fn say(&mut self, ha: usize, message: impl Into<Message>) {
            let message: Message = message.into();
            self.has[ha]
                .1
                .send(message.id(), &message.encode())
                .unwrap();
            self.settle();
        }

        fn take(&mut self, ha: usize) -> Vec<Message> {
            self.settle();
            std::mem::take(&mut self.has[ha].2)
        }

        fn settle(&mut self) {
            for _ in 0..1000 {
                let mut moved = false;
                for i in 0..self.has.len() {
                    let id = self.has[i].0;
                    let out = self.has[i].1.take_output();
                    if !out.is_empty() {
                        self.proxy.client_input(id, &out, self.now);
                        moved = true;
                    }
                    let out = self.proxy.client_output(id);
                    if !out.is_empty() {
                        self.has[i]
                            .1
                            .input(&out)
                            .expect("the proxy speaks the protocol");
                        moved = true;
                    }
                    while let Some((kind, payload)) = self.has[i].1.next_message() {
                        if let Some(message) = Message::decode(kind, &payload).unwrap() {
                            self.has[i].2.push(message);
                        }
                        moved = true;
                    }
                }
                if !moved {
                    return;
                }
            }
            panic!("never went quiet");
        }

        fn advance(&mut self, by: Duration) {
            self.now += by;
            self.proxy.handle_timeout(self.now);
            self.settle();
        }
    }

    #[test]
    fn home_assistant_is_told_what_this_device_is() {
        let mut rig = Rig::new();
        let ha = rig.connect();
        rig.say(ha, m::DeviceInfoRequest::default());
        let Some(Message::DeviceInfoResponse(info)) = rig.take(ha).pop() else {
            panic!("no device info");
        };
        assert_eq!(info.name, "lumatv-ble");
        assert_eq!(info.mac_address, "AA:BB:CC:00:11:22");
        assert_eq!(info.bluetooth_mac_address, "84:A3:29:B9:4D:43");
        assert!(info.api_encryption_supported);
        let flags = info.bluetooth_proxy_feature_flags;
        assert_eq!(
            flags,
            feature::PASSIVE_SCAN | feature::RAW_ADVERTISEMENTS | feature::STATE_AND_MODE
        );
        // No connections, pairing or cache: LG's service keeps the controller.
        assert_eq!(
            flags & (feature::ACTIVE_CONNECTIONS | feature::PAIRING | feature::REMOTE_CACHING),
            0
        );
        rig.say(ha, m::DeviceCapabilitiesRequest::default());
        let Some(Message::DeviceCapabilitiesResponse(c)) = rig.take(ha).pop() else {
            panic!("no capabilities");
        };
        assert_eq!(c.bluetooth_feature_flags, flags);
        rig.say(ha, m::ListEntitiesRequest::default());
        assert_eq!(
            rig.take(ha),
            vec![Message::ListEntitiesDoneResponse(Default::default())]
        );
    }

    #[test]
    fn a_wrong_key_is_refused_with_the_reason_and_a_plaintext_client_is_told_to_encrypt() {
        let mut rig = Rig::new();
        let id = rig.proxy.add_client().unwrap();
        let mut wrong = Ha::new([0x11; 32], [3; 32]);
        rig.proxy.client_input(id, &wrong.take_output(), rig.now);
        let out = rig.proxy.client_output(id);
        let _ = wrong.input(&out);
        let more = wrong.take_output();
        rig.proxy.client_input(id, &more, rig.now);
        let out = rig.proxy.client_output(id);
        let _ = wrong.input(&out);
        assert_eq!(wrong.rejected(), Some("Handshake MAC failure"));
        assert!(rig.proxy.client_closing(id));
        assert_eq!(rig.proxy.take_actions(), vec![Action::CloseClient(id)]);

        let id = rig.proxy.add_client().unwrap();
        rig.proxy.client_input(id, &[0x00, 0x00, 0x00], rig.now);
        assert!(rig.proxy.client_closing(id));
        assert!(
            !rig.proxy.client_output(id).is_empty(),
            "the 'wants encryption' frame"
        );
    }

    #[test]
    fn only_four_clients_are_kept() {
        let mut rig = Rig::new();
        for _ in 0..MAX_CLIENTS {
            assert!(rig.proxy.add_client().is_some());
        }
        assert!(rig.proxy.add_client().is_none());
        let first = rig.proxy.client_ids()[0];
        rig.proxy.client_gone(first);
        assert!(rig.proxy.add_client().is_some());
    }

    #[test]
    fn subscribing_wants_the_radio_and_is_told_the_scanner_state() {
        let mut rig = Rig::new();
        let ha = rig.connect();
        assert!(!rig.proxy.wanted());
        rig.say(
            ha,
            m::SubscribeBluetoothLEAdvertisementsRequest { flags: 1 },
        );
        assert!(rig.proxy.wanted());
        // Idle until the runner says otherwise.
        assert_eq!(
            rig.take(ha),
            vec![Message::BluetoothScannerStateResponse(
                m::BluetoothScannerStateResponse {
                    state: scanner::IDLE,
                    mode: scanner::PASSIVE,
                    configured_mode: scanner::PASSIVE,
                }
            )]
        );
        rig.proxy.set_scan_state(ScanState::Running);
        rig.settle();
        let Some(Message::BluetoothScannerStateResponse(s)) = rig.take(ha).pop() else {
            panic!("no state");
        };
        assert_eq!((s.state, s.mode), (scanner::RUNNING, scanner::PASSIVE));
        // The same state is not told twice.
        rig.proxy.set_scan_state(ScanState::Running);
        assert!(rig.take(ha).is_empty());
        // A mode Home Assistant asks for is kept as configured, and the scanner stays passive.
        rig.say(
            ha,
            m::BluetoothScannerSetModeRequest {
                mode: scanner::ACTIVE,
            },
        );
        let Some(Message::BluetoothScannerStateResponse(s)) = rig.take(ha).pop() else {
            panic!("no state after the mode");
        };
        assert_eq!(
            (s.mode, s.configured_mode),
            (scanner::PASSIVE, scanner::ACTIVE)
        );
        rig.say(
            ha,
            m::UnsubscribeBluetoothLEAdvertisementsRequest::default(),
        );
        assert!(!rig.proxy.wanted());
    }

    #[test]
    fn advertisements_come_in_batches_of_sixteen_or_after_a_tenth_of_a_second() {
        let mut rig = Rig::new();
        let ha = rig.connect();
        rig.say(
            ha,
            m::SubscribeBluetoothLEAdvertisementsRequest { flags: 1 },
        );
        rig.take(ha);
        for i in 0..16u64 {
            rig.proxy.heard(advert(0xaabb_ccdd_ee00 + i), rig.now);
        }
        let batches = rig.take(ha);
        let [Message::BluetoothLERawAdvertisementsResponse(b)] = &batches[..] else {
            panic!("one batch of sixteen, got {batches:?}");
        };
        assert_eq!(b.advertisements.len(), 16);
        assert_eq!(b.advertisements[3], advert(0xaabb_ccdd_ee03));
        // One alone waits for company, then goes.
        rig.proxy.heard(advert(1), rig.now);
        assert!(rig.take(ha).is_empty());
        assert_eq!(
            rig.proxy.next_deadline(),
            Some(rig.now + ADVERTISEMENT_WAIT)
        );
        rig.advance(Duration::from_millis(99));
        assert!(rig.take(ha).is_empty());
        rig.advance(Duration::from_millis(1));
        let batches = rig.take(ha);
        assert!(
            matches!(&batches[..], [Message::BluetoothLERawAdvertisementsResponse(b)] if b.advertisements.len() == 1)
        );
        assert_eq!(rig.proxy.next_deadline(), None);
        assert_eq!(rig.proxy.status().advertisements, 17);
    }

    #[test]
    fn nothing_is_kept_for_nobody_and_the_newest_subscriber_wins() {
        let mut rig = Rig::new();
        let a = rig.connect();
        rig.proxy.heard(advert(1), rig.now);
        assert_eq!(rig.proxy.status().advertisements, 0);
        rig.say(a, m::SubscribeBluetoothLEAdvertisementsRequest { flags: 1 });
        let b = rig.connect();
        rig.say(b, m::SubscribeBluetoothLEAdvertisementsRequest { flags: 1 });
        rig.take(a);
        rig.take(b);
        for i in 0..16 {
            rig.proxy.heard(advert(i), rig.now);
        }
        assert!(rig.take(a).is_empty());
        assert_eq!(rig.take(b).len(), 1);
        // The subscriber goes: the radio is not wanted any more, and what was half a batch is not sent to the next.
        rig.proxy.heard(advert(99), rig.now);
        let id = rig.has[b].0;
        rig.proxy.client_gone(id);
        assert!(!rig.proxy.wanted() && rig.proxy.next_deadline().is_none());
    }

    #[test]
    fn a_subscriber_that_does_not_read_costs_dropped_advertisements_not_memory() {
        let mut rig = Rig::new();
        let ha = rig.connect();
        rig.say(
            ha,
            m::SubscribeBluetoothLEAdvertisementsRequest { flags: 1 },
        );
        // Never read: output piles up in the proxy until the backlog is over, then advertisements are dropped.
        let id = rig.has[ha].0;
        for i in 0..100_000u64 {
            rig.proxy.heard(
                m::RawAdvertisement {
                    address: i,
                    rssi: -60,
                    address_type: 0,
                    data: vec![0; 62],
                },
                rig.now,
            );
            if rig.proxy.status().dropped > 0 {
                break;
            }
        }
        let status = rig.proxy.status();
        assert!(status.dropped > 0);
        assert!(rig.proxy.clients[&id].out.len() < MAX_BACKLOG + 4096);
    }

    #[test]
    fn a_request_to_connect_is_answered_not_connected_and_the_key_cannot_be_changed() {
        let mut rig = Rig::new();
        let ha = rig.connect();
        rig.say(
            ha,
            m::BluetoothDeviceRequest {
                address: 0xaabb_ccdd_ee01,
                request_type: m::request_type::CONNECT_V3_WITH_CACHE,
                has_address_type: true,
                address_type: 0,
            },
        );
        assert_eq!(
            rig.take(ha),
            vec![Message::BluetoothDeviceConnectionResponse(
                m::BluetoothDeviceConnectionResponse {
                    address: 0xaabb_ccdd_ee01,
                    connected: false,
                    mtu: 0,
                    error: 0,
                }
            )]
        );
        rig.say(ha, m::NoiseEncryptionSetKeyRequest { key: vec![1; 32] });
        assert_eq!(
            rig.take(ha),
            vec![Message::NoiseEncryptionSetKeyResponse(
                m::NoiseEncryptionSetKeyResponse { success: false }
            )]
        );
        rig.say(ha, m::SubscribeBluetoothConnectionsFreeRequest::default());
        let Some(Message::BluetoothConnectionsFreeResponse(free)) = rig.take(ha).pop() else {
            panic!("no slots message");
        };
        assert_eq!((free.free, free.limit), (0, 0));
    }

    #[test]
    fn a_disconnect_request_is_answered_and_ends_the_connection() {
        let mut rig = Rig::new();
        let ha = rig.connect();
        rig.say(ha, m::DisconnectRequest::default());
        assert_eq!(
            rig.take(ha),
            vec![Message::DisconnectResponse(Default::default())]
        );
        assert!(rig.proxy.client_closing(rig.has[ha].0));
    }
}
