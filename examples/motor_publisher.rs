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

use bytemuck::{Pod, Zeroable};
use mq_ipc::wire::WireTx;
use std::{io, thread, time::Duration};

/// Example typed message for a motor state.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MotorState {
    pub position: f32,
    pub velocity: f32,
    pub torque: f32,
}

fn main() -> io::Result<()> {
    // Create a wire-aware topic:
    // - local topic: "/motor/state"
    // - internal TX topic: "/ipc_tx" (inside the lib)
    let motor = WireTx::<MotorState>::new("/example_motor_state", 4)?;

    println!("motor_publisher started (local + mirrored to /ipc_tx).");

    let mut angle: f32 = 0.0;
    let mut vel: f32 = 1.0;

    loop {
        angle += 0.1;
        vel += 0.05;

        let msg = MotorState {
            position: angle,
            velocity: vel,
            torque: 0.42,
        };

        motor.publish(&msg)?;
        println!(
            "Published MotorState: pos={:.3}, vel={:.3}, tq={:.3}",
            msg.position, msg.velocity, msg.torque
        );

        thread::sleep(Duration::from_millis(10));
    }
}
