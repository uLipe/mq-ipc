/*
MIT License
Copyright (c) 2025 Felipe Neves

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
*/

use libc::{self, mqd_t};
use std::{
    ffi::CString,
    io,
    os::raw::{c_char, c_long},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
};

use bytemuck::{Pod, Zeroable};

pub const MSG_PAYLOAD_SIZE: usize = 240;

const MSG_TYPE_SHUTDOWN: u16 = 0xFFFF;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct MsgHeader {
    pub msg_type: u16,
    pub len: u16,
}

/// Complete raw message sent over an mqueue.
#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct Msg {
    pub hdr: MsgHeader,
    pub payload: [u8; MSG_PAYLOAD_SIZE],
}

impl Msg {
    /// Create a new raw message from a type and arbitrary bytes.
    pub fn new(msg_type: u16, data: &[u8]) -> Self {
        let mut msg = Msg {
            hdr: MsgHeader {
                msg_type,
                len: data.len().min(MSG_PAYLOAD_SIZE) as u16,
            },
            payload: [0u8; MSG_PAYLOAD_SIZE],
        };
        let n = msg.hdr.len as usize;
        msg.payload[..n].copy_from_slice(&data[..n]);
        msg
    }
}

type Callback = Box<dyn Fn(Msg) + Send + Sync + 'static>;

/// A system-wide topic backed by POSIX mqueue (`mqueue`).
///
/// Multiple processes can open the same name (e.g. "/topic.motor_state")
/// and publish to / subscribe from it. Inside this process, you can
/// register multiple callbacks that are invoked by a background worker
/// thread whenever a message arrives.
pub struct MqTopic {
    name: String,
    mqd: mqd_t,
    subs: Arc<Mutex<Vec<Callback>>>,
    running: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl MqTopic {
    /// Create or open a topic backed by a POSIX mqueue.
    ///
    /// - `name` must start with '/' (POSIX requirement).
    /// - `maxmsg` is the maximum number of messages that can be queued.
    pub fn new(name: &str, maxmsg: c_long) -> io::Result<Self> {
        let cname = CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid queue name"))?;

        let mut attr: libc::mq_attr = unsafe { std::mem::zeroed() };
        attr.mq_flags = 0;
        attr.mq_maxmsg = maxmsg;
        attr.mq_msgsize = std::mem::size_of::<Msg>() as c_long;
        attr.mq_curmsgs = 0;

        let mqd = unsafe {
            libc::mq_open(
                cname.as_ptr(),
                libc::O_CREAT | libc::O_RDWR,
                0o666,
                &mut attr,
            )
        };

        if mqd == -1 {
            return Err(io::Error::last_os_error());
        }

        let subs = Arc::new(Mutex::new(Vec::<Callback>::new()));
        let running = Arc::new(AtomicBool::new(true));
        let worker = Self::spawn_worker(mqd, Arc::clone(&subs), Arc::clone(&running));

        Ok(MqTopic {
            name: name.to_string(),
            mqd,
            subs,
            running,
            worker: Some(worker),
        })
    }

    pub fn open_existing(name: &str) -> io::Result<Option<Self>> {
        let cname = CString::new(name)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid queue name"))?;

        let mqd = unsafe {
            libc::mq_open(
                cname.as_ptr(),
                libc::O_RDWR,
                0o660,
                std::ptr::null_mut::<libc::mq_attr>(),
            )
        };

        if mqd == -1 {
            let err = io::Error::last_os_error();
            if let Some(code) = err.raw_os_error() {
                if code == libc::ENOENT {
                    return Ok(None)
                }
            }
            return Err(err);
        }

        let subs = Arc::new(Mutex::new(Vec::<Callback>::new()));
        let running = Arc::new(AtomicBool::new(true));
        let worker = Self::spawn_worker(mqd, Arc::clone(&subs), Arc::clone(&running));

        Ok(Some(MqTopic {
            name: name.to_string(),
            mqd,
            subs,
            running,
            worker: Some(worker),
        }))
    }

    fn spawn_worker(
        mqd: mqd_t,
        subs: Arc<Mutex<Vec<Callback>>>,
        running: Arc<AtomicBool>,
    ) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let mut buf = [0u8; std::mem::size_of::<Msg>()];

            loop {
                let mut prio: u32 = 0;
                let ret = unsafe {
                    libc::mq_receive(
                        mqd,
                        buf.as_mut_ptr() as *mut c_char,
                        buf.len(),
                        &mut prio as *mut u32,
                    )
                };

                if ret < 0 {
                    let err = io::Error::last_os_error();
                    if let Some(code) = err.raw_os_error() {
                        match code {
                            libc::EINTR => {
                                // sinal interrompeu; se já mandaram parar, sai
                                if !running.load(Ordering::Relaxed) {
                                    break;
                                }
                                continue;
                            }
                            libc::EBADF => {
                                // fila foi fechada: hora de sair
                                break;
                            }
                            _ => {
                                eprintln!("mq_receive error: {err}");
                                if !running.load(Ordering::Relaxed) {
                                    break;
                                }
                                continue;
                            }
                        }
                    }
                    break;
                }

                // SAFETY: buffer contém Msg válido
                let msg: Msg = unsafe { std::ptr::read(buf.as_ptr() as *const Msg) };

                if msg.hdr.msg_type == MSG_TYPE_SHUTDOWN
                    && !running.load(Ordering::Relaxed)
                {
                    break;
                }

                let guard = subs.lock().unwrap();
                for cb in guard.iter() {
                    cb(msg);
                }
            }
        })
    }

    /// Register a callback to be invoked whenever a message arrives.
    pub fn subscribe<F>(&self, f: F)
    where
        F: Fn(Msg) + Send + Sync + 'static,
    {
        let mut guard = self.subs.lock().unwrap();
        guard.push(Box::new(f));
    }

    /// Publish a raw message to this topic with a given priority.
    pub fn publish(&self, msg: &Msg, prio: u32) -> io::Result<()> {
        let data_ptr = msg as *const Msg as *const c_char;
        let len = std::mem::size_of::<Msg>();
        let rc = unsafe { libc::mq_send(self.mqd, data_ptr, len, prio) };
        if rc == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    /// Get the POSIX mqueue name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Get the raw mqd_t for advanced usage.
    pub fn raw_mqd(&self) -> mqd_t {
        self.mqd
    }
}

impl Drop for MqTopic {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        let shutdown = Msg::new(MSG_TYPE_SHUTDOWN, &[]);

        unsafe {
            let data_ptr = &shutdown as *const Msg as *const c_char;
            let rc = libc::mq_send(
                self.mqd,
                data_ptr,
                std::mem::size_of::<Msg>(),
                0,
            );
            if rc == -1 {
                eprintln!("mq_send shutdown failed: {}", io::Error::last_os_error());
            }

            libc::mq_close(self.mqd);
        }

        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }

        // unlink opcional, se você quiser limpar o nome automaticamente
        // if let Ok(cname) = CString::new(self.name.clone()) {
        //     unsafe { libc::mq_unlink(cname.as_ptr()); }
        // }
    }
}

/// Strongly-typed IPC topic built on top of `MqTopic`.
///
/// T must be Pod + Zeroable so it can be safely mapped to raw bytes.
pub struct Topic<T>
where
    T: Pod + Zeroable + Send + Sync + 'static,
{
    inner: MqTopic,
    _marker: std::marker::PhantomData<T>,
}

impl<T> Topic<T>
where
    T: Pod + Zeroable + Send + Sync + 'static,
{
    /// Create or open a typed topic.
    pub fn new(name: &str, maxmsg: c_long) -> io::Result<Self> {
        let inner = MqTopic::new(name, maxmsg)?;
        Ok(Self {
            inner,
            _marker: std::marker::PhantomData,
        })
    }

    /// Subscribe with a callback that receives `T` directly.
    pub fn subscribe<F>(&self, f: F)
    where
        F: Fn(T) + Send + Sync + 'static,
    {
        self.inner.subscribe(move |msg: Msg| {
            let mut buf = vec![0u8; std::mem::size_of::<T>()];
            let n = std::cmp::min(msg.hdr.len as usize, buf.len());
            buf[..n].copy_from_slice(&msg.payload[..n]);
            let value: T = *bytemuck::from_bytes::<T>(&buf[..]);
            f(value);
        });
    }

    /// Publish a typed value as a message with the given `msg_type` and priority.
    pub fn publish(&self, value: &T, msg_type: u16, prio: u32) -> io::Result<()> {
        let bytes: &[u8] = bytemuck::bytes_of(value);
        let msg = Msg::new(msg_type, bytes);
        self.inner.publish(&msg, prio)
    }

    /// Expose the underlying raw topic.
    pub fn raw(&self) -> &MqTopic {
        &self.inner
    }
}

/// Wire-related utilities and the internal TX mirroring.
pub mod wire {
    /*
    MIT License
    (repeated for module clarity, optional)
    */

    use super::Topic;
    use bytemuck::{Pod, Zeroable};
    use std::io;
    use std::marker::PhantomData;
    use std::os::raw::c_long;

    /// Internal, fixed name for the wire TX topic.
    pub const IPC_TX_TOPIC_NAME: &str = "/ipc_tx";

    /// Maximum topic name length stored in the wire packet.
    pub const WIRE_MAX_TOPIC: usize = 64;

    /// Maximum payload size carried in a wire packet.
    pub const WIRE_MAX_PAYLOAD: usize = 128;

    /// Generic wire packet: topic name (as bytes) + payload bytes.
    ///
    /// The actual topic name length is in `topic_len`, and the payload
    /// length is in `payload_len`. Both are truncated to their respective
    /// max sizes if needed.
    #[repr(C)]
    #[derive(Copy, Clone, Debug, Pod, Zeroable)]
    pub struct WirePacket {
        pub payload_len: u16,
        pub topic_len: u8,
        pub reserved: u8,
        pub topic: [u8; WIRE_MAX_TOPIC],
        pub data: [u8; WIRE_MAX_PAYLOAD],
    }

    impl WirePacket {
        /// Try to decode the topic name as UTF-8.
        /// Returns an empty string on invalid UTF-8.
        pub fn topic_name(&self) -> String {
            let len = self.topic_len as usize;
            let len = len.min(WIRE_MAX_TOPIC);
            match std::str::from_utf8(&self.topic[..len]) {
                Ok(s) => s.to_string(),
                Err(_) => String::new(),
            }
        }
    }

    /// WireTx<T>:
    /// - publishes T to the local topic
    /// - mirrors a serialized T as WirePacket into the *internal* TX topic ("/ipc_tx"),
    ///   including the topic name as a UTF-8 string in the packet.
    pub struct WireTx<T>
    where
        T: Pod + Zeroable + Send + Sync + 'static,
    {
        local: Topic<T>,         // e.g. "/motor/state"
        tx: Topic<WirePacket>,   // always "/ipc_tx" under the hood
        topic_name: String,      // stored so we can serialize it on every publish
        _marker: PhantomData<T>,
    }

    impl<T> WireTx<T>
    where
        T: Pod + Zeroable + Send + Sync + 'static,
    {
        /// Creates a wire-aware topic:
        /// - `local_topic_name`: application topic (e.g. "/motor/state")
        /// The TX topic is always the internal "/ipc_tx".
        pub fn new(local_topic_name: &str, maxmsg: c_long) -> io::Result<Self> {
            let local = Topic::<T>::new(local_topic_name, maxmsg)?;
            let tx = Topic::<WirePacket>::new(IPC_TX_TOPIC_NAME, maxmsg)?;

            Ok(Self {
                local,
                tx,
                topic_name: local_topic_name.to_string(),
                _marker: PhantomData,
            })
        }

        /// Publish:
        /// 1) local T on its normal topic
        /// 2) mirror as WirePacket on the internal "/ipc_tx".
        ///
        /// The WirePacket will carry:
        /// - topic name as UTF-8 (truncated to WIRE_MAX_TOPIC)
        /// - serialized T bytes (truncated to WIRE_MAX_PAYLOAD)
        pub fn publish(&self, value: &T) -> io::Result<()> {
            // 1) local publish
            self.local.publish(value, 1, 0)?;

            // 2) serialize T + topic name into WirePacket on "/ipc_tx"
            let topic_bytes = self.topic_name.as_bytes();
            let tlen = topic_bytes.len().min(WIRE_MAX_TOPIC);

            let raw = bytemuck::bytes_of(value);
            let plen = raw.len().min(WIRE_MAX_PAYLOAD);

            let mut pkt = WirePacket {
                topic_len: tlen as u8,
                payload_len: plen as u16,
                reserved: 0,
                topic: [0u8; WIRE_MAX_TOPIC],
                data: [0u8; WIRE_MAX_PAYLOAD],
            };

            pkt.topic[..tlen].copy_from_slice(&topic_bytes[..tlen]);
            pkt.data[..plen].copy_from_slice(&raw[..plen]);

            self.tx.publish(&pkt, 0, 0)
        }

        /// If you still want to subscribe locally:
        pub fn local(&self) -> &Topic<T> {
            &self.local
        }
    }

    /// Helper to open the internal TX topic as a typed topic of WirePacket.
    ///
    /// This is what a "router" process would use to listen for frames
    /// that need to be sent over a physical link (serial, CAN, etc).
    pub fn open_ipc_tx(maxmsg: c_long) -> io::Result<Topic<WirePacket>> {
        Topic::<WirePacket>::new(IPC_TX_TOPIC_NAME, maxmsg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::{Pod, Zeroable};
    use libc;
    use std::{
        ffi::CString,
        sync::{Arc, Mutex},
        thread,
        time::Duration,
    };

    #[repr(C)]
    #[derive(Copy, Clone, Debug, Pod, Zeroable, PartialEq)]
    struct TestMsg {
        a: u32,
        b: u32,
    }

    fn unlink_queue(name: &str) {
        if let Ok(cname) = CString::new(name) {
            unsafe {
                let rc = libc::mq_unlink(cname.as_ptr());
                if rc == -1 {
                    // Ignore ENOENT.
                }
            }
        }
    }

    #[test]
    fn create_topic_and_publish() {
        let topic_name = format!("/mq_ipc_test_create_{}", std::process::id());

        {
            let topic: Topic<TestMsg> =
                Topic::new(&topic_name, 4).expect("failed to create topic");

            let msg = TestMsg { a: 1, b: 2 };
            topic
                .publish(&msg, 1, 0)
                .expect("failed to publish to topic");
        }

        unlink_queue(&topic_name);
    }

    #[test]
    fn publish_and_receive_with_subscriber() {
        let topic_name = format!("/mq_ipc_test_sub_{}", std::process::id());

        {
            let topic: Topic<TestMsg> =
                Topic::new(&topic_name, 4).expect("failed to create topic");

            let received: Arc<Mutex<Vec<TestMsg>>> = Arc::new(Mutex::new(Vec::new()));
            let received_clone = Arc::clone(&received);

            topic.subscribe(move |m: TestMsg| {
                received_clone.lock().unwrap().push(m);
            });

            let msg = TestMsg { a: 10, b: 20 };
            topic
                .publish(&msg, 1, 0)
                .expect("failed to publish to topic");

            for _ in 0..50 {
                {
                    let guard = received.lock().unwrap();
                    if !guard.is_empty() {
                        assert_eq!(guard[0], msg);
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }

            let guard = received.lock().unwrap();
            assert_eq!(guard.len(), 1, "expected exactly one received message");
            assert_eq!(guard[0], msg);
        }

        unlink_queue(&topic_name);
    }

    // #[test]
    // fn wiretx_produces_expected_wirepacket() {
    //     let local_topic = format!("/mq_ipc_test_wiretx_{}", std::process::id());

    //     {
    //         let wire_tx =
    //             wire::WireTx::<TestMsg>::new(&local_topic, 4).expect("failed to create WireTx");

    //         let tx_topic =
    //             wire::open_ipc_tx(4).expect("failed to open internal ipc_tx topic");

    //         let received: Arc<Mutex<Vec<wire::WirePacket>>> =
    //             Arc::new(Mutex::new(Vec::new()));
    //         let received_clone = Arc::clone(&received);

    //         tx_topic.subscribe(move |pkt: wire::WirePacket| {
    //             received_clone.lock().unwrap().push(pkt);
    //         });

    //         let msg = TestMsg {
    //             a: 0xDEAD_BEEF,
    //             b: 0x1234_5678,
    //         };

    //         wire_tx
    //             .publish(&msg)
    //             .expect("failed to publish via WireTx");

    //         for _ in 0..50 {
    //             {
    //                 let guard = received.lock().unwrap();
    //                 if !guard.is_empty() {
    //                     break;
    //                 }
    //             }
    //             thread::sleep(Duration::from_millis(10));
    //         }

    //         let guard = received.lock().unwrap();
    //         assert!(
    //             !guard.is_empty(),
    //             "expected at least one WirePacket in /ipc_tx"
    //         );

    //         let pkt = guard.last().unwrap();

    //         assert_eq!(
    //             pkt.topic_name(),
    //             local_topic,
    //             "WirePacket topic name mismatch"
    //         );

    //         let expected_bytes = bytemuck::bytes_of(&msg);
    //         let plen = pkt.payload_len as usize;
    //         assert!(
    //             plen <= expected_bytes.len(),
    //             "payload_len is larger than expected struct size"
    //         );
    //         assert_eq!(
    //             &pkt.data[..plen],
    //             &expected_bytes[..plen],
    //             "WirePacket payload bytes do not match TestMsg"
    //         );
    //     }

    //     unlink_queue(&local_topic);
    //     unlink_queue(wire::IPC_TX_TOPIC_NAME);
    // }
}
