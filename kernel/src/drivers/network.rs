//! Network Driver Stack
//!
//! Architecture:
//! - RTL8139 PCI NIC driver (RX ring + 4 TX descriptors)
//! - smoltcp TCP/IP stack on top (ARP, IPv4, ICMP, TCP, UDP, DHCP, DNS)
//!
//! The old hand-rolled TCP/ARP/DNS implementation was replaced by smoltcp:
//! it provides retransmission, reassembly, window management and RFC-correct
//! state machines, which the previous implementation lacked (connections died
//! after the RX ring wrapped and any packet loss wedged the stack).
//!
//! Locking rules:
//! - `RTL8139` (driver) and `STACK` (smoltcp interface + sockets) are separate
//!   spin mutexes. `STACK` may lock `RTL8139` (via Device tokens), never the
//!   other way around.
//! - Public blocking helpers never hold either lock while yielding/polling.

use alloc::vec;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use spin::Mutex;

use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{self, Device, DeviceCapabilities, Medium};
use smoltcp::socket::{dhcpv4, dns, icmp, tcp, udp};
use smoltcp::time::Instant;
use smoltcp::wire::{
    DnsQueryType, EthernetAddress, HardwareAddress, Icmpv4Packet, Icmpv4Repr, IpAddress, IpCidr,
    IpEndpoint, IpListenEndpoint, Ipv4Address, Ipv4Cidr,
};

use crate::arch::x86_64::{inb, inl, inw, outb, outl, outw};

// ─── PCI ─────────────────────────────────────────────────────────────────────

const PCI_CONFIG_ADDRESS: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

const RTL8139_VENDOR_ID: u16 = 0x10EC;
const RTL8139_DEVICE_ID: u16 = 0x8139;

// ─── RTL8139 registers ───────────────────────────────────────────────────────

const REG_IDR0: u16 = 0x00;
const REG_TSD0: u16 = 0x10;
const REG_TSAD0: u16 = 0x20;
const REG_RBSTART: u16 = 0x30;
const REG_CMD: u16 = 0x37;
const REG_CAPR: u16 = 0x38;
const REG_IMR: u16 = 0x3C;
const REG_ISR: u16 = 0x3E;
const REG_TCR: u16 = 0x40;
const REG_RCR: u16 = 0x44;
const REG_CONFIG1: u16 = 0x52;

const CMD_RESET: u8 = 1 << 4;
const CMD_RX_ENABLE: u8 = 1 << 3;
const CMD_TX_ENABLE: u8 = 1 << 2;
const CMD_RX_EMPTY: u8 = 1 << 0;

const ISR_RX_ERR: u16 = 1 << 1;
const ISR_TX_ERR: u16 = 1 << 3;

/// TSD bits: OWN is set by the NIC once the packet has been moved to FIFO,
/// meaning the descriptor buffer can be reused.
const TSD_OWN: u32 = 1 << 13;

/// Hardware RX ring: RCR RBLEN=00 → 8192 bytes. The NIC is configured with
/// WRAP=1, so a packet crossing the 8K boundary is written *contiguously*
/// past the end of the ring (up to MTU bytes of slack are required).
const RX_RING_SIZE: usize = 8192;
/// Allocated RX buffer: ring + 16 header slack + 2048 wrap slack (3 pages).
const RX_ALLOC_SIZE: usize = 12288;

const MAX_FRAME_SIZE: usize = 1600;

// ─── Static IPv4 fallback config (QEMU user-mode defaults) ──────────────────

const DEFAULT_IP: [u8; 4] = [10, 0, 2, 15];
const DEFAULT_NETMASK: [u8; 4] = [255, 255, 255, 0];
const DEFAULT_GATEWAY: [u8; 4] = [10, 0, 2, 2];
const DEFAULT_DNS: [u8; 4] = [10, 0, 2, 3];

#[derive(Clone, Copy)]
struct NetConfig {
    ip: [u8; 4],
    netmask: [u8; 4],
    gateway: [u8; 4],
    dns: [u8; 4],
}

impl NetConfig {
    const fn default() -> Self {
        Self {
            ip: DEFAULT_IP,
            netmask: DEFAULT_NETMASK,
            gateway: DEFAULT_GATEWAY,
            dns: DEFAULT_DNS,
        }
    }
}

// ─── Globals ─────────────────────────────────────────────────────────────────

static RTL8139: Mutex<Option<Rtl8139>> = Mutex::new(None);
static NET_CONFIG: Mutex<NetConfig> = Mutex::new(NetConfig::default());

static RX_PACKETS: AtomicU64 = AtomicU64::new(0);
static TX_PACKETS: AtomicU64 = AtomicU64::new(0);
static RX_ERRORS: AtomicU64 = AtomicU64::new(0);
static TX_ERRORS: AtomicU64 = AtomicU64::new(0);
static ICMP_ECHO_RX: AtomicU64 = AtomicU64::new(0);
static ICMP_ECHO_TX: AtomicU64 = AtomicU64::new(0);
static PING_SEQ: AtomicU16 = AtomicU16::new(1);
static TCP_SRC_PORT_SEQ: AtomicU16 = AtomicU16::new(49152);
/// Bumped every time DHCP (re)configures the interface.
static DHCP_CONFIG_GEN: AtomicU64 = AtomicU64::new(0);

const PING_IDENT: u16 = 0xC077;

fn now() -> Instant {
    Instant::from_millis(crate::proc::scheduler::ticks() as i64)
}

// ─── PCI access ──────────────────────────────────────────────────────────────

fn pci_read_u32(bus: u8, slot: u8, func: u8, offset: u8) -> u32 {
    let address = (1u32 << 31)
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((offset as u32) & 0xFC);

    outl(PCI_CONFIG_ADDRESS, address);
    inl(PCI_CONFIG_DATA)
}

fn pci_write_u16(bus: u8, slot: u8, func: u8, offset: u8, value: u16) {
    let aligned = offset & 0xFC;
    let shift = ((offset & 0x02) * 8) as u32;
    let mut current = pci_read_u32(bus, slot, func, aligned);
    current &= !(0xFFFF << shift);
    current |= (value as u32) << shift;

    let address = (1u32 << 31)
        | ((bus as u32) << 16)
        | ((slot as u32) << 11)
        | ((func as u32) << 8)
        | ((aligned as u32) & 0xFC);

    outl(PCI_CONFIG_ADDRESS, address);
    outl(PCI_CONFIG_DATA, current);
}

#[derive(Clone, Copy)]
struct PciLocation {
    bus: u8,
    slot: u8,
    func: u8,
}

fn find_rtl8139() -> Option<PciLocation> {
    for bus in 0..=255 {
        for slot in 0..32 {
            let vendor_device = pci_read_u32(bus, slot, 0, 0x00);
            if vendor_device == 0xFFFF_FFFF {
                continue;
            }

            let vendor = (vendor_device & 0xFFFF) as u16;
            let device = ((vendor_device >> 16) & 0xFFFF) as u16;
            if vendor == RTL8139_VENDOR_ID && device == RTL8139_DEVICE_ID {
                return Some(PciLocation { bus, slot, func: 0 });
            }
        }
    }
    None
}

fn io_read_u8(io_base: u16, reg: u16) -> u8 {
    inb(io_base + reg)
}

fn io_read_u16(io_base: u16, reg: u16) -> u16 {
    inw(io_base + reg)
}

fn io_read_u32(io_base: u16, reg: u16) -> u32 {
    inl(io_base + reg)
}

fn io_write_u8(io_base: u16, reg: u16, value: u8) {
    outb(io_base + reg, value);
}

fn io_write_u16(io_base: u16, reg: u16, value: u16) {
    outw(io_base + reg, value);
}

fn io_write_u32(io_base: u16, reg: u16, value: u32) {
    outl(io_base + reg, value);
}

// ─── RTL8139 driver ──────────────────────────────────────────────────────────

struct Rtl8139 {
    io_base: u16,
    irq: u8,
    mac: [u8; 6],
    rx_buffer_phys: u64,
    rx_offset: usize,
    tx_buffers_phys: [u64; 4],
    tx_cur: usize,
}

impl Rtl8139 {
    fn init() -> Result<Self, &'static str> {
        let loc = find_rtl8139().ok_or("RTL8139 not found")?;

        let bar0 = pci_read_u32(loc.bus, loc.slot, loc.func, 0x10);
        if bar0 == 0 || (bar0 & 0x1) == 0 {
            return Err("RTL8139 BAR0 not I/O-mapped");
        }
        let io_base = (bar0 & 0xFFFC) as u16;

        let irq_line = (pci_read_u32(loc.bus, loc.slot, loc.func, 0x3C) & 0xFF) as u8;

        // Enable I/O space + bus mastering
        let command = (pci_read_u32(loc.bus, loc.slot, loc.func, 0x04) & 0xFFFF) as u16;
        let command = command | (1 << 0) | (1 << 2);
        pci_write_u16(loc.bus, loc.slot, loc.func, 0x04, command);

        io_write_u8(io_base, REG_CONFIG1, 0x00);

        io_write_u8(io_base, REG_CMD, CMD_RESET);
        for _ in 0..100_000 {
            if io_read_u8(io_base, REG_CMD) & CMD_RESET == 0 {
                break;
            }
        }
        if io_read_u8(io_base, REG_CMD) & CMD_RESET != 0 {
            return Err("RTL8139 reset timeout");
        }

        let rx_buffer_phys = crate::mm::physical::alloc_frames(3).ok_or("No memory for RX ring")?;
        let mut tx_buffers_phys = [0u64; 4];
        for entry in &mut tx_buffers_phys {
            *entry = crate::mm::physical::alloc_frame().ok_or("No memory for TX buffer")?;
        }

        let mut nic = Self {
            io_base,
            irq: irq_line,
            mac: [0; 6],
            rx_buffer_phys,
            rx_offset: 0,
            tx_buffers_phys,
            tx_cur: 0,
        };

        nic.program(true);

        for (i, byte) in nic.mac.iter_mut().enumerate() {
            *byte = io_read_u8(io_base, REG_IDR0 + i as u16);
        }

        Ok(nic)
    }

    /// Program RBSTART/TSADs/RCR/TCR and enable RX+TX.
    /// Used both at init and to recover from an RX ring desync.
    fn program(&mut self, first: bool) {
        if !first {
            // Soft-stop the receiver before rewriting RBSTART
            io_write_u8(self.io_base, REG_CMD, CMD_TX_ENABLE);
        }

        io_write_u32(self.io_base, REG_RBSTART, self.rx_buffer_phys as u32);
        for (i, addr) in self.tx_buffers_phys.iter().enumerate() {
            io_write_u32(self.io_base, REG_TSAD0 + (i as u16 * 4), *addr as u32);
        }

        // Mask all interrupts (pure polling mode), ack anything pending
        io_write_u16(self.io_base, REG_IMR, 0x0000);
        io_write_u16(self.io_base, REG_ISR, 0xFFFF);

        // RCR: accept broadcast/multicast/phys-match/all-phys, WRAP=1,
        // max DMA burst 1024 (6<<8), RBLEN=00 → 8192-byte ring.
        io_write_u32(self.io_base, REG_RCR, 0x0000_000F | (1 << 7) | (6 << 8));
        io_write_u32(self.io_base, REG_TCR, 0x0300_0700);

        io_write_u8(self.io_base, REG_CMD, CMD_RX_ENABLE | CMD_TX_ENABLE);

        self.rx_offset = 0;
    }

    /// Recover from a corrupted RX ring: restart the receiver from offset 0.
    fn recover_rx(&mut self) {
        RX_ERRORS.fetch_add(1, Ordering::Relaxed);
        self.program(false);
    }

    fn send_frame(&mut self, frame: &[u8]) -> Result<(), &'static str> {
        if frame.len() > 1792 {
            return Err("Frame too large");
        }

        let tx_idx = self.tx_cur % 4;
        let tsd_reg = REG_TSD0 + (tx_idx as u16 * 4);

        // Wait (bounded) until the descriptor is free. OWN=1 means the NIC
        // has finished DMA-ing this buffer to its FIFO; descriptors also read
        // as free right after reset.
        let mut free = false;
        for _ in 0..200_000 {
            if io_read_u32(self.io_base, tsd_reg) & TSD_OWN != 0 {
                free = true;
                break;
            }
            core::hint::spin_loop();
        }
        if !free {
            TX_ERRORS.fetch_add(1, Ordering::Relaxed);
            return Err("TX descriptor busy");
        }

        let tx_addr = self.tx_buffers_phys[tx_idx] as *mut u8;
        unsafe {
            core::ptr::copy_nonoverlapping(frame.as_ptr(), tx_addr, frame.len());
        }

        // Ethernet minimum frame: pad to 60 bytes (NIC appends CRC)
        let mut len = frame.len();
        if len < 60 {
            unsafe {
                core::ptr::write_bytes(tx_addr.add(len), 0, 60 - len);
            }
            len = 60;
        }

        io_write_u32(self.io_base, tsd_reg, len as u32);
        self.tx_cur = (self.tx_cur + 1) % 4;
        TX_PACKETS.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Pull one frame out of the RX ring. Returns None when the ring is empty.
    fn recv_frame(&mut self) -> Option<Vec<u8>> {
        if io_read_u8(self.io_base, REG_CMD) & CMD_RX_EMPTY != 0 {
            return None;
        }

        let base_ptr = self.rx_buffer_phys as *const u8;
        // 4-byte header written by the NIC: status u16, length u16 (LE).
        // WRAP=1 guarantees the header + packet are contiguous in memory
        // (the NIC writes past the 8K boundary into the slack area).
        let status = unsafe {
            u16::from_le_bytes([*base_ptr.add(self.rx_offset), *base_ptr.add(self.rx_offset + 1)])
        };
        let length = unsafe {
            u16::from_le_bytes([
                *base_ptr.add(self.rx_offset + 2),
                *base_ptr.add(self.rx_offset + 3),
            ])
        } as usize;

        // status bit 0 = ROK. Anything else, or an insane length, means the
        // ring is desynced — restart the receiver instead of guessing.
        if status & 0x01 == 0 || length < 8 || length > MAX_FRAME_SIZE + 4 {
            self.recover_rx();
            return None;
        }

        let frame_len = length - 4; // strip CRC
        let mut frame = vec![0u8; frame_len];
        unsafe {
            core::ptr::copy_nonoverlapping(
                base_ptr.add(self.rx_offset + 4),
                frame.as_mut_ptr(),
                frame_len,
            );
        }

        // Advance read pointer: header(4) + packet, dword-aligned, modulo the
        // 8K ring (the hardware wraps the *write* pointer back to 0 too).
        self.rx_offset = (self.rx_offset + 4 + length + 3) & !3;
        if self.rx_offset >= RX_RING_SIZE {
            self.rx_offset -= RX_RING_SIZE;
        }
        io_write_u16(
            self.io_base,
            REG_CAPR,
            (self.rx_offset as u16).wrapping_sub(16),
        );

        RX_PACKETS.fetch_add(1, Ordering::Relaxed);
        Some(frame)
    }

    fn handle_interrupt(&mut self) {
        let isr = io_read_u16(self.io_base, REG_ISR);
        if isr == 0 {
            return;
        }

        io_write_u16(self.io_base, REG_ISR, isr);

        if (isr & ISR_RX_ERR) != 0 {
            RX_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        if (isr & ISR_TX_ERR) != 0 {
            TX_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
    }
}

// ─── smoltcp Device implementation ───────────────────────────────────────────

/// smoltcp device backed by the global RTL8139 driver.
struct CottonDevice;

struct CottonRxToken(Vec<u8>);
struct CottonTxToken;

impl phy::RxToken for CottonRxToken {
    fn consume<R, F>(self, f: F) -> R
    where
        F: FnOnce(&[u8]) -> R,
    {
        f(&self.0)
    }
}

impl phy::TxToken for CottonTxToken {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buf = vec![0u8; len];
        let result = f(&mut buf);
        if let Some(ref mut nic) = *RTL8139.lock() {
            let _ = nic.send_frame(&buf);
        }
        result
    }
}

impl Device for CottonDevice {
    type RxToken<'a>
        = CottonRxToken
    where
        Self: 'a;
    type TxToken<'a>
        = CottonTxToken
    where
        Self: 'a;

    fn receive(&mut self, _timestamp: Instant) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        let frame = RTL8139.lock().as_mut().and_then(|nic| nic.recv_frame())?;
        Some((CottonRxToken(frame), CottonTxToken))
    }

    fn transmit(&mut self, _timestamp: Instant) -> Option<Self::TxToken<'_>> {
        if RTL8139.lock().is_some() {
            Some(CottonTxToken)
        } else {
            None
        }
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut caps = DeviceCapabilities::default();
        caps.medium = Medium::Ethernet;
        caps.max_transmission_unit = 1514;
        caps.max_burst_size = Some(4);
        caps
    }
}

// ─── smoltcp stack ───────────────────────────────────────────────────────────

struct NetStack {
    iface: Interface,
    device: CottonDevice,
    sockets: SocketSet<'static>,
    tcp_handle: SocketHandle,
    udp_handle: SocketHandle,
    udp_bound_port: u16,
    dns_handle: SocketHandle,
    dhcp_handle: SocketHandle,
    icmp_handle: SocketHandle,
}

static STACK: Mutex<Option<NetStack>> = Mutex::new(None);

fn ip4(octets: [u8; 4]) -> Ipv4Address {
    Ipv4Address::new(octets[0], octets[1], octets[2], octets[3])
}

fn prefix_len(mask: [u8; 4]) -> u8 {
    u32::from_be_bytes(mask).count_ones() as u8
}

impl NetStack {
    fn new(mac: [u8; 6]) -> Self {
        let mut device = CottonDevice;

        let mut config = Config::new(HardwareAddress::Ethernet(EthernetAddress(mac)));
        let mut seed = crate::proc::scheduler::ticks()
            ^ ((mac[5] as u64) << 40)
            ^ 0x9e37_79b9_7f4a_7c15;
        if seed == 0 {
            seed = 1;
        }
        config.random_seed = seed;

        let mut iface = Interface::new(config, &mut device, now());

        let cfg = *NET_CONFIG.lock();
        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::new(IpAddress::Ipv4(ip4(cfg.ip)), prefix_len(cfg.netmask)));
        });
        let _ = iface.routes_mut().add_default_ipv4_route(ip4(cfg.gateway));

        let mut sockets = SocketSet::new(Vec::new());

        let tcp_rx = tcp::SocketBuffer::new(vec![0u8; 65536]);
        let tcp_tx = tcp::SocketBuffer::new(vec![0u8; 16384]);
        let mut tcp_socket = tcp::Socket::new(tcp_rx, tcp_tx);
        tcp_socket.set_nagle_enabled(false);
        let tcp_handle = sockets.add(tcp_socket);

        let udp_rx = udp::PacketBuffer::new(
            vec![udp::PacketMetadata::EMPTY; 8],
            vec![0u8; 8192],
        );
        let udp_tx = udp::PacketBuffer::new(
            vec![udp::PacketMetadata::EMPTY; 8],
            vec![0u8; 8192],
        );
        let udp_handle = sockets.add(udp::Socket::new(udp_rx, udp_tx));

        let dns_queries: Vec<Option<dns::DnsQuery>> = (0..4).map(|_| None).collect();
        let dns_handle = sockets.add(dns::Socket::new(
            &[IpAddress::Ipv4(ip4(cfg.dns))],
            dns_queries,
        ));

        let dhcp_handle = sockets.add(dhcpv4::Socket::new());

        let icmp_rx = icmp::PacketBuffer::new(
            vec![icmp::PacketMetadata::EMPTY; 8],
            vec![0u8; 4096],
        );
        let icmp_tx = icmp::PacketBuffer::new(
            vec![icmp::PacketMetadata::EMPTY; 8],
            vec![0u8; 4096],
        );
        let mut icmp_socket = icmp::Socket::new(icmp_rx, icmp_tx);
        let _ = icmp_socket.bind(icmp::Endpoint::Ident(PING_IDENT));
        let icmp_handle = sockets.add(icmp_socket);

        Self {
            iface,
            device,
            sockets,
            tcp_handle,
            udp_handle,
            udp_bound_port: 0,
            dns_handle,
            dhcp_handle,
            icmp_handle,
        }
    }

    fn apply_config(&mut self) {
        let cfg = *NET_CONFIG.lock();
        self.iface.update_ip_addrs(|addrs| {
            addrs.clear();
            let _ = addrs.push(IpCidr::new(IpAddress::Ipv4(ip4(cfg.ip)), prefix_len(cfg.netmask)));
        });
        self.iface.routes_mut().remove_default_ipv4_route();
        let _ = self.iface.routes_mut().add_default_ipv4_route(ip4(cfg.gateway));
        self.sockets
            .get_mut::<dns::Socket>(self.dns_handle)
            .update_servers(&[IpAddress::Ipv4(ip4(cfg.dns))]);
    }

    /// One stack iteration: drive smoltcp, apply DHCP events, count pings.
    fn drive(&mut self) {
        let timestamp = now();
        let _ = self.iface.poll(timestamp, &mut self.device, &mut self.sockets);

        // DHCP events (only meaningful after dhcp_configure() resets the socket)
        let event = self
            .sockets
            .get_mut::<dhcpv4::Socket>(self.dhcp_handle)
            .poll();
        if let Some(event) = event {
            match event {
                dhcpv4::Event::Configured(config) => {
                    {
                        let mut cfg = NET_CONFIG.lock();
                        cfg.ip = config.address.address().octets();
                        cfg.netmask = Ipv4Cidr::new(Ipv4Address::UNSPECIFIED, config.address.prefix_len())
                            .netmask()
                            .octets();
                        if let Some(router) = config.router {
                            cfg.gateway = router.octets();
                        }
                        if let Some(dns_server) = config.dns_servers.first() {
                            cfg.dns = dns_server.octets();
                        }
                    }
                    self.apply_config();
                    DHCP_CONFIG_GEN.fetch_add(1, Ordering::SeqCst);
                }
                dhcpv4::Event::Deconfigured => {}
            }
        }

        // Drain ICMP echo replies so netstat counters stay meaningful
        let icmp_socket = self.sockets.get_mut::<icmp::Socket>(self.icmp_handle);
        while icmp_socket.can_recv() {
            if let Ok((payload, _addr)) = icmp_socket.recv() {
                if let Ok(packet) = Icmpv4Packet::new_checked(payload) {
                    if packet.msg_type() == smoltcp::wire::Icmpv4Message::EchoReply {
                        ICMP_ECHO_RX.fetch_add(1, Ordering::Relaxed);
                    }
                }
            } else {
                break;
            }
        }
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

pub fn init() {
    match Rtl8139::init() {
        Ok(driver) => {
            let mac = driver.mac;
            crate::kprintln!(
                "[NET] RTL8139 up: io={:#x} irq={} mac={:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
                driver.io_base,
                driver.irq,
                mac[0],
                mac[1],
                mac[2],
                mac[3],
                mac[4],
                mac[5]
            );
            *RTL8139.lock() = Some(driver);

            let stack = NetStack::new(mac);
            *STACK.lock() = Some(stack);

            let cfg = *NET_CONFIG.lock();
            crate::kprintln!(
                "[NET] smoltcp up: IPv4={}.{}.{}.{} gw={}.{}.{}.{} dns={}.{}.{}.{}",
                cfg.ip[0],
                cfg.ip[1],
                cfg.ip[2],
                cfg.ip[3],
                cfg.gateway[0],
                cfg.gateway[1],
                cfg.gateway[2],
                cfg.gateway[3],
                cfg.dns[0],
                cfg.dns[1],
                cfg.dns[2],
                cfg.dns[3]
            );
        }
        Err(err) => {
            crate::kprintln!("[NET] No RTL8139 network device: {}", err);
        }
    }
}

pub fn is_available() -> bool {
    RTL8139.lock().is_some()
}

pub fn handle_interrupt() {
    if let Some(mut guard) = RTL8139.try_lock() {
        if let Some(ref mut nic) = *guard {
            nic.handle_interrupt();
        }
    }
}

/// Drive the network stack one step. Safe to call from anywhere that does not
/// already hold the STACK or RTL8139 locks.
pub fn poll() {
    if let Some(mut guard) = STACK.try_lock() {
        if let Some(stack) = guard.as_mut() {
            stack.drive();
        }
    }
}

pub fn mac() -> Option<[u8; 6]> {
    RTL8139.lock().as_ref().map(|nic| nic.mac)
}

pub fn ip() -> [u8; 4] {
    NET_CONFIG.lock().ip
}

pub fn netmask() -> [u8; 4] {
    NET_CONFIG.lock().netmask
}

pub fn gateway() -> [u8; 4] {
    NET_CONFIG.lock().gateway
}

pub fn dns_server() -> [u8; 4] {
    NET_CONFIG.lock().dns
}

fn reconfigure<F: FnOnce(&mut NetConfig)>(f: F) {
    f(&mut NET_CONFIG.lock());
    if let Some(stack) = STACK.lock().as_mut() {
        stack.apply_config();
    }
}

pub fn set_ip(new_ip: [u8; 4]) {
    reconfigure(|cfg| cfg.ip = new_ip);
}

pub fn set_netmask(new_netmask: [u8; 4]) {
    reconfigure(|cfg| cfg.netmask = new_netmask);
}

pub fn set_gateway(new_gateway: [u8; 4]) {
    reconfigure(|cfg| cfg.gateway = new_gateway);
}

pub fn set_dns(new_dns: [u8; 4]) {
    reconfigure(|cfg| cfg.dns = new_dns);
}

/// smoltcp manages ARP internally; kept for API compatibility.
pub fn request_arp(_ip: [u8; 4]) -> Result<(), &'static str> {
    if is_available() {
        poll();
        Ok(())
    } else {
        Err("network unavailable")
    }
}

/// ARP/neighbor cache is internal to smoltcp now; nothing to report.
pub fn arp_entries() -> [([u8; 4], [u8; 6], bool); 8] {
    [([0u8; 4], [0u8; 6], false); 8]
}

pub fn ping(target: [u8; 4]) -> Result<(), &'static str> {
    let seq = PING_SEQ.fetch_add(1, Ordering::Relaxed);

    {
        let mut guard = STACK.lock();
        let stack = guard.as_mut().ok_or("network unavailable")?;
        let socket = stack.sockets.get_mut::<icmp::Socket>(stack.icmp_handle);
        if !socket.can_send() {
            return Err("icmp socket busy");
        }

        let payload = b"CottonOS ping";
        let repr = Icmpv4Repr::EchoRequest {
            ident: PING_IDENT,
            seq_no: seq,
            data: payload,
        };
        let addr = IpAddress::Ipv4(ip4(target));
        let buf = socket
            .send(repr.buffer_len(), addr)
            .map_err(|_| "icmp send failed")?;
        let mut packet = Icmpv4Packet::new_unchecked(buf);
        repr.emit(&mut packet, &DeviceCapabilities::default().checksum);
    }

    ICMP_ECHO_TX.fetch_add(1, Ordering::Relaxed);
    poll();
    Ok(())
}

// ─── TCP (single client connection, same API as before) ─────────────────────

pub fn tcp_connect(target: [u8; 4], port: u16) -> Result<(), &'static str> {
    let local_port = {
        let p = TCP_SRC_PORT_SEQ.fetch_add(1, Ordering::Relaxed);
        if p < 49152 {
            TCP_SRC_PORT_SEQ.store(49153, Ordering::Relaxed);
            49152
        } else {
            p
        }
    };

    let mut guard = STACK.lock();
    let stack = guard.as_mut().ok_or("network unavailable")?;
    let NetStack {
        iface,
        sockets,
        tcp_handle,
        ..
    } = stack;
    let socket = sockets.get_mut::<tcp::Socket>(*tcp_handle);

    // Make sure any previous connection is fully torn down
    if socket.is_open() || socket.state() != tcp::State::Closed {
        socket.abort();
    }

    socket
        .connect(
            iface.context(),
            IpEndpoint::new(IpAddress::Ipv4(ip4(target)), port),
            local_port,
        )
        .map_err(|_| "tcp connect failed")?;

    drop(guard);
    poll();
    Ok(())
}

/// True while the connection can still produce data for the reader:
/// established, or closed by the peer but with unread bytes buffered.
pub fn tcp_is_connected() -> bool {
    let mut guard = STACK.lock();
    if let Some(stack) = guard.as_mut() {
        let socket = stack.sockets.get_mut::<tcp::Socket>(stack.tcp_handle);
        socket.may_recv() || socket.recv_queue() > 0
    } else {
        false
    }
}

/// Send all of `data`, blocking (with polls + yields) until buffered.
pub fn tcp_send(data: &[u8]) -> Result<(), &'static str> {
    let mut sent = 0usize;
    let deadline = crate::proc::scheduler::ticks() + 10_000;

    while sent < data.len() {
        {
            let mut guard = STACK.lock();
            let stack = guard.as_mut().ok_or("network unavailable")?;
            let socket = stack.sockets.get_mut::<tcp::Socket>(stack.tcp_handle);

            if !socket.may_send() {
                return Err("tcp not connected");
            }
            match socket.send_slice(&data[sent..]) {
                Ok(n) => sent += n,
                Err(_) => return Err("tcp send failed"),
            }
        }

        poll();
        if sent < data.len() {
            if crate::proc::scheduler::ticks() > deadline {
                return Err("tcp send timeout");
            }
            crate::task::yield_to_main();
        }
    }

    poll();
    Ok(())
}

pub fn tcp_read_into(buf: &mut [u8]) -> usize {
    if buf.is_empty() {
        return 0;
    }

    let mut guard = STACK.lock();
    if let Some(stack) = guard.as_mut() {
        let socket = stack.sockets.get_mut::<tcp::Socket>(stack.tcp_handle);
        if socket.can_recv() {
            return socket.recv_slice(buf).unwrap_or(0);
        }
    }
    0
}

pub fn tcp_read() -> Option<([u8; 1024], usize)> {
    let mut out = [0u8; 1024];
    let len = tcp_read_into(&mut out);
    if len == 0 {
        None
    } else {
        Some((out, len))
    }
}

pub fn tcp_close() -> Result<(), &'static str> {
    {
        let mut guard = STACK.lock();
        let stack = guard.as_mut().ok_or("network unavailable")?;
        let socket = stack.sockets.get_mut::<tcp::Socket>(stack.tcp_handle);
        socket.close();
    }

    // Give the FIN a chance to go out; don't block long. The next
    // tcp_connect() aborts the socket anyway if it is still lingering.
    for _ in 0..4 {
        poll();
    }
    Ok(())
}

// ─── UDP ─────────────────────────────────────────────────────────────────────

pub fn udp_send(
    dst_ip: [u8; 4],
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Result<(), &'static str> {
    {
        let mut guard = STACK.lock();
        let stack = guard.as_mut().ok_or("network unavailable")?;
        let handle = stack.udp_handle;
        let bound = stack.udp_bound_port;
        let socket = stack.sockets.get_mut::<udp::Socket>(handle);

        if bound != src_port {
            socket.close();
            socket
                .bind(IpListenEndpoint {
                    addr: None,
                    port: src_port,
                })
                .map_err(|_| "udp bind failed")?;
            stack.udp_bound_port = src_port;
        }

        let socket = stack.sockets.get_mut::<udp::Socket>(handle);
        socket
            .send_slice(
                payload,
                IpEndpoint::new(IpAddress::Ipv4(ip4(dst_ip)), dst_port),
            )
            .map_err(|_| "udp send failed")?;
    }

    poll();
    Ok(())
}

pub fn udp_recv() -> Option<([u8; 4], u16, u16, [u8; 1024], usize)> {
    poll();
    let mut guard = STACK.lock();
    let stack = guard.as_mut()?;
    let local_port = stack.udp_bound_port;
    let socket = stack.sockets.get_mut::<udp::Socket>(stack.udp_handle);

    let mut out = [0u8; 1024];
    match socket.recv() {
        Ok((data, meta)) => {
            let len = core::cmp::min(data.len(), out.len());
            out[..len].copy_from_slice(&data[..len]);
            let src = match meta.endpoint.addr {
                IpAddress::Ipv4(v4) => v4.octets(),
            };
            Some((src, meta.endpoint.port, local_port, out, len))
        }
        Err(_) => None,
    }
}

// ─── DHCP ────────────────────────────────────────────────────────────────────

pub fn dhcp_configure() -> Result<(), &'static str> {
    let generation = DHCP_CONFIG_GEN.load(Ordering::SeqCst);

    {
        let mut guard = STACK.lock();
        let stack = guard.as_mut().ok_or("network unavailable")?;
        let socket = stack.sockets.get_mut::<dhcpv4::Socket>(stack.dhcp_handle);
        socket.reset();
    }

    let deadline = crate::proc::scheduler::ticks() + 8000;
    while crate::proc::scheduler::ticks() < deadline {
        poll();
        if DHCP_CONFIG_GEN.load(Ordering::SeqCst) != generation {
            return Ok(());
        }
        crate::task::yield_to_main();
    }

    Err("dhcp timeout")
}

// ─── DNS ─────────────────────────────────────────────────────────────────────

pub fn dns_resolve_a(host: &str) -> Result<[u8; 4], &'static str> {
    if host.is_empty() || host.len() > 240 {
        return Err("invalid host name");
    }

    // Literal IPv4 doesn't need DNS
    if let Some(addr) = parse_ipv4_literal(host) {
        return Ok(addr);
    }

    let query = {
        let mut guard = STACK.lock();
        let stack = guard.as_mut().ok_or("network unavailable")?;
        let NetStack {
            iface,
            sockets,
            dns_handle,
            ..
        } = stack;
        let socket = sockets.get_mut::<dns::Socket>(*dns_handle);
        socket
            .start_query(iface.context(), host, DnsQueryType::A)
            .map_err(|_| "dns query failed to start")?
    };

    let deadline = crate::proc::scheduler::ticks() + 8000;
    loop {
        poll();

        {
            let mut guard = STACK.lock();
            let stack = guard.as_mut().ok_or("network unavailable")?;
            let socket = stack.sockets.get_mut::<dns::Socket>(stack.dns_handle);
            match socket.get_query_result(query) {
                Ok(addrs) => {
                    for addr in addrs {
                        #[allow(irrefutable_let_patterns)]
                        if let IpAddress::Ipv4(v4) = addr {
                            return Ok(v4.octets());
                        }
                    }
                    return Err("dns: no A record");
                }
                Err(dns::GetQueryResultError::Pending) => {}
                Err(_) => return Err("dns query failed"),
            }
        }

        if crate::proc::scheduler::ticks() >= deadline {
            let mut guard = STACK.lock();
            if let Some(stack) = guard.as_mut() {
                let socket = stack.sockets.get_mut::<dns::Socket>(stack.dns_handle);
                socket.cancel_query(query);
            }
            return Err("dns timeout");
        }

        crate::task::yield_to_main();
    }
}

fn parse_ipv4_literal(s: &str) -> Option<[u8; 4]> {
    let mut it = s.splitn(5, '.');
    let a = it.next()?.parse::<u8>().ok()?;
    let b = it.next()?.parse::<u8>().ok()?;
    let c = it.next()?.parse::<u8>().ok()?;
    let d = it.next()?.parse::<u8>().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some([a, b, c, d])
}

// ─── Stats ───────────────────────────────────────────────────────────────────

pub fn stats() -> (u64, u64, u64, u64, u64, u64) {
    (
        RX_PACKETS.load(Ordering::Relaxed),
        TX_PACKETS.load(Ordering::Relaxed),
        RX_ERRORS.load(Ordering::Relaxed),
        TX_ERRORS.load(Ordering::Relaxed),
        ICMP_ECHO_RX.load(Ordering::Relaxed),
        ICMP_ECHO_TX.load(Ordering::Relaxed),
    )
}
