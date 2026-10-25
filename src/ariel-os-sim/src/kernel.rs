//! The connection to the simulator's kernel, and what the node knows from it: its clock,
//! its input pins, and its identity.
//!
//! A node is one thread. Everything here is behind one critical section, and every request
//! blocks until the kernel answers, so the node does nothing else meanwhile (lockstep).
//!
//! A broken connection (a socket error, a message that doesn't decode, a message out of
//! turn) panics: the simulator records that as the node's failure.

#![allow(
    clippy::missing_panics_doc,
    reason = "every call panics on a broken kernel connection, as the module says"
)]

use core::cell::RefCell;
use core::task::Waker;
use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};

use crate::wire::generated::{HelloMsg, HelloVersion, IdleMsg};
use crate::wire::{Edge, Hello, Idle, KernelMessage, NodeMessage, Reply, Start};
use critical_section::Mutex;
use socket2::{Domain, SockAddr, Socket, Type};

/// Environment variable naming the kernel's socket.
const SOCKET_ENV: &str = "ARIEL_SIM_SOCKET";
/// Environment variable holding the token the kernel gave this process.
const TOKEN_ENV: &str = "ARIEL_SIM_TOKEN";

/// Nanoseconds per time-driver tick (1 MHz, D8).
pub(crate) const NS_PER_TICK: u64 = 1000;

/// An input pin as the node last heard of it.
#[derive(Default)]
struct PinState {
    level: bool,
    /// Counts of rising and falling edges, so an edge wait can tell one happened.
    rising: u32,
    falling: u32,
    wakers: Vec<Waker>,
}

/// What the start message said.
#[allow(
    dead_code,
    reason = "the seed and the storage range are read only with their features"
)]
pub(crate) struct Identity {
    pub(crate) node: u64,
    pub(crate) boot: u64,
    pub(crate) eui64: [u8; 8],
    pub(crate) rng_seed: u64,
    pub(crate) storage: core::ops::Range<u32>,
}

pub(crate) struct Kernel {
    sock: Socket,
    buf: Vec<u8>,
    /// The node's clock, in virtual nanoseconds.
    now: u64,
    pins: BTreeMap<&'static str, PinState>,
    pub(crate) timers: embassy_time_queue_utils::Queue,
    pub(crate) identity: Identity,
}

static KERNEL: Mutex<RefCell<Option<Kernel>>> = Mutex::new(RefCell::new(None));

/// Runs `f` on the kernel connection.
///
/// # Panics
///
/// Before [`connect`], or if called from inside `f`.
pub(crate) fn with<R>(f: impl FnOnce(&mut Kernel) -> R) -> R {
    critical_section::with(|cs| {
        let mut k = KERNEL.borrow_ref_mut(cs);
        f(k.as_mut()
            .expect("the kernel connection is set up by hal::init()"))
    })
}

/// Connects to the kernel and takes the start message. Exits the process when not started
/// by the simulator.
pub(crate) fn connect() {
    let Some(path) = std::env::var_os(SOCKET_ENV) else {
        eprintln!(
            "this is an ariel-sim firmware binary (board sim): it is started by the \
             simulator, which sets {SOCKET_ENV}"
        );
        std::process::exit(2);
    };
    let token = std::env::var(TOKEN_ENV).unwrap_or_default();
    let sock = Socket::new(Domain::UNIX, Type::SEQPACKET, None)
        .unwrap_or_else(|e| panic!("ariel-sim: socket: {e}"));
    let addr = SockAddr::unix(&path).unwrap_or_else(|e| panic!("ariel-sim: socket path: {e}"));
    sock.connect(&addr)
        .unwrap_or_else(|e| panic!("ariel-sim: connecting to {}: {e}", path.display()));
    let mut k = Kernel {
        sock,
        buf: Vec::with_capacity(crate::wire::MAX_MESSAGE),
        now: 0,
        pins: BTreeMap::new(),
        timers: embassy_time_queue_utils::Queue::new(),
        identity: Identity {
            node: 0,
            boot: 0,
            eui64: [0; 8],
            rng_seed: 0,
            storage: 0..0,
        },
    };
    k.send(&NodeMessage::Hello(Hello {
        msg: HelloMsg,
        version: HelloVersion,
        token,
    }));
    let KernelMessage::Start(start) = k.recv() else {
        panic!("ariel-sim: expected start from the kernel");
    };
    k.start(&start);
    critical_section::with(|cs| *KERNEL.borrow_ref_mut(cs) = Some(k));
}

impl Kernel {
    fn start(&mut self, s: &Start) {
        let eui64 = <[u8; 8]>::try_from(s.eui64.as_slice())
            .unwrap_or_else(|_| panic!("ariel-sim: start.eui64 is not 8 bytes"));
        let range = |v: u64| u32::try_from(v).expect("ariel-sim: storage range above 4 GiB");
        self.now = s.t;
        self.identity = Identity {
            node: s.node,
            boot: s.boot,
            eui64,
            rng_seed: s.rng_seed,
            storage: range(s.storage_offset)..range(s.storage_offset + s.storage_len),
        };
    }

    /// The node's clock in nanoseconds.
    pub(crate) fn now(&self) -> u64 {
        self.now
    }

    /// The node's clock in time-driver ticks.
    pub(crate) fn ticks(&self) -> u64 {
        self.now / NS_PER_TICK
    }

    fn send(&mut self, msg: &NodeMessage) {
        // Everything the node printed belongs before this message (sim-node's console).
        let _ = std::io::stdout().flush();
        let _ = std::io::stderr().flush();
        crate::wire::encode(msg, &mut self.buf).unwrap_or_else(|e| panic!("ariel-sim: {e}"));
        let sent = self
            .sock
            .send(&self.buf)
            .unwrap_or_else(|e| panic!("ariel-sim: sending to the kernel: {e}"));
        assert_eq!(sent, self.buf.len(), "ariel-sim: short send");
    }

    fn recv(&mut self) -> KernelMessage {
        // One read is one packet; one byte more than the largest message shows truncation.
        self.buf.clear();
        self.buf.resize(crate::wire::MAX_MESSAGE + 1, 0);
        let n = (&self.sock)
            .read(&mut self.buf)
            .unwrap_or_else(|e| panic!("ariel-sim: receiving from the kernel: {e}"));
        if n == 0 {
            // The kernel went away: nothing more will happen to this node.
            std::process::exit(1);
        }
        let packet = self.buf.get(..n).unwrap_or_default();
        let msg = crate::wire::decode::<KernelMessage>(packet)
            .unwrap_or_else(|e| panic!("ariel-sim: {e}"));
        if let KernelMessage::Shutdown(_) = msg {
            let _ = std::io::stdout().flush();
            std::process::exit(0);
        }
        msg
    }

    /// Sends a request and returns the kernel's reply; the clock moves to the reply's
    /// time and its edges are applied.
    ///
    /// # Panics
    ///
    /// If the kernel refuses the request: that is a bug in the board or this HAL.
    pub(crate) fn request(&mut self, msg: &NodeMessage) -> Reply {
        self.send(msg);
        let KernelMessage::Reply(reply) = self.recv() else {
            panic!("ariel-sim: expected a reply from the kernel");
        };
        if let Some(error) = &reply.error {
            panic!("ariel-sim: the kernel refused a request: {error}");
        }
        self.now = self.now.max(reply.t);
        self.apply(&reply.edges);
        reply
    }

    /// Tells the kernel nothing is ready to run, and waits until it says to go on.
    pub(crate) fn idle(&mut self, next_wake_ticks: u64) {
        let next_wake = next_wake_ticks.checked_mul(NS_PER_TICK);
        let t = self.now;
        self.send(&NodeMessage::Idle(Idle {
            msg: IdleMsg,
            t,
            next_wake,
        }));
        let KernelMessage::Advance(advance) = self.recv() else {
            panic!("ariel-sim: expected advance from the kernel");
        };
        self.now = self.now.max(advance.t);
        self.apply(&advance.edges);
    }

    fn apply(&mut self, edges: &[Edge]) {
        for e in edges {
            let Some(pin) = self.pins.get_mut(e.pin.as_str()) else {
                continue;
            };
            if pin.level != e.level {
                pin.level = e.level;
                if e.level {
                    pin.rising = pin.rising.wrapping_add(1);
                } else {
                    pin.falling = pin.falling.wrapping_add(1);
                }
            }
            for w in pin.wakers.drain(..) {
                w.wake();
            }
        }
    }

    /// Records an input pin and its level.
    pub(crate) fn track_input(&mut self, pin: &'static str, level: bool) {
        let p = self.pins.entry(pin).or_default();
        p.level = level;
    }

    /// The level an input pin was last given.
    pub(crate) fn level(&self, pin: &'static str) -> bool {
        self.pins.get(pin).is_some_and(|p| p.level)
    }

    /// The edge counts of an input pin: (rising, falling).
    pub(crate) fn edges(&self, pin: &'static str) -> (u32, u32) {
        self.pins.get(pin).map_or((0, 0), |p| (p.rising, p.falling))
    }

    /// Wakes `waker` at the pin's next edge.
    pub(crate) fn wake_on_edge(&mut self, pin: &'static str, waker: &Waker) {
        let p = self.pins.entry(pin).or_default();
        if !p.wakers.iter().any(|w| w.will_wake(waker)) {
            p.wakers.push(waker.clone());
        }
    }
}
