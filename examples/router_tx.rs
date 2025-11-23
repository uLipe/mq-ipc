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

use bytemuck::Pod;
use mqueue_ipc::wire::{open_ipc_tx, WirePacket};
use std::{io, thread, time::Duration};

fn send_over_wire(pkt: &WirePacket) {
    println!(
        "[router_tx] WIRE TX: hash=0x{:08X}, len={}",
        pkt.topic_hash, pkt.len
    );

    let header_size = std::mem::size_of::<WirePacket>() - WirePacket::data.len();
    let bytes: &[u8] = bytemuck::bytes_of(pkt);
    let total = header_size + pkt.len as usize;

    print!("  raw: ");
    for b in &bytes[..total] {
        print!("{:02X} ", b);
    }
    println!();
}

fn main() -> io::Result<()> {
    // Open the internal TX topic that the IPC uses to mirror all wire-aware publishes.
    let tx_topic = open_ipc_tx(32)?;

    println!("router_tx started. Listening on /ipc_tx...");

    tx_topic.subscribe(|pkt: WirePacket| {
        // In a real system, this is where you would write `pkt.data[..pkt.len]`
        // to a serial port, CAN frame, or some other physical transport.
        send_over_wire(&pkt);
    });

    loop {
        thread::sleep(Duration::from_secs(1));
    }
}
