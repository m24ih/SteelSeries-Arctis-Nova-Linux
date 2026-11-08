use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use usbip_audio::uac2::{Uac2Config, Uac2Device, Uac2Streams};
use usbip_audio::{MonotonicClock, Server};

#[derive(Default)]
struct Counter(AtomicU64);

impl Uac2Streams for Counter {
    fn playback(&self, _frame: u64, samples: &[i32]) {
        self.0
            .fetch_add((samples.len() / 2) as u64, Ordering::Relaxed);
    }

    fn capture(&self, _frame: u64, samples: &mut [i32]) {
        samples.fill(0);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = Server::start(0, Arc::new(MonotonicClock::new()))?;
    let mut counters = Vec::new();
    for (index, name) in ["Game", "Chat"].iter().enumerate() {
        let config = Uac2Config {
            vendor_id: 0x1209,
            product_id: 0x0001,
            bcd_device: 0x0100,
            manufacturer: "Arctis prototype".into(),
            product: format!("Arctis {name} prototype"),
            serial: format!("arctis-prototype-{}", name.to_lowercase()),
            sample_rate: 48000,
            playback_channels: 2,
            capture_channels: 0,
            playback_name: Some(format!("Arctis {name}")),
            capture_name: None,
            capture_connector: false,
        };
        let counter = Arc::new(Counter::default());
        let device = Uac2Device::new(config, counter.clone()).map_err(std::io::Error::other)?;
        server.add(&format!("1-{}", index + 1), Arc::new(device));
        counters.push(counter);
    }
    println!(
        "USB/IP server on localhost:{}; devices 1-1 (Game), 1-2 (Chat).",
        server.port()
    );
    println!("Prototype discards audio; it does not play through the headset.");
    println!("Use Ctrl+C to stop.");
    loop {
        std::thread::sleep(Duration::from_secs(5));
        println!(
            "Received frames: game={}, chat={}",
            counters[0].0.load(Ordering::Relaxed),
            counters[1].0.load(Ordering::Relaxed)
        );
    }
}
