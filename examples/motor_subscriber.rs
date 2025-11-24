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

use mq_ipc::Topic;
use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct MotorState {
    pub position: f32,
    pub velocity: f32,
    pub torque:  f32,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let topic_name = "/example_motor_state";
    let topic: Topic<MotorState> = Topic::new(topic_name, 8)?;

    println!("motor_subscriber: listening on topic {}", topic_name);

    topic.subscribe(|state: MotorState| {
        println!(
            "[motor_subscriber] received: position={:.3}, velocity={:.3}, torque={:.3}",
            state.position, state.velocity, state.torque
        );
    });

    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
