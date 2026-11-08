//! A short-lived, silent render session for manual routing integration tests.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::error::Error;
use std::time::Duration;

fn main() -> Result<(), Box<dyn Error>> {
    let seconds: u64 = std::env::args().nth(1).as_deref().unwrap_or("30").parse()?;
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("No default output device")?;
    let supported = device.default_output_config()?;
    let config = supported.config();
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            &config,
            |data: &mut [f32], _| data.fill(0.0),
            |err| eprintln!("{err}"),
            None,
        )?,
        cpal::SampleFormat::I16 => device.build_output_stream(
            &config,
            |data: &mut [i16], _| data.fill(0),
            |err| eprintln!("{err}"),
            None,
        )?,
        cpal::SampleFormat::U16 => device.build_output_stream(
            &config,
            |data: &mut [u16], _| data.fill(1 << 15),
            |err| eprintln!("{err}"),
            None,
        )?,
        format => return Err(format!("Unsupported output format: {format:?}").into()),
    };
    stream.play()?;
    println!("Silent test session running for {seconds} seconds");
    std::thread::sleep(Duration::from_secs(seconds));
    Ok(())
}
