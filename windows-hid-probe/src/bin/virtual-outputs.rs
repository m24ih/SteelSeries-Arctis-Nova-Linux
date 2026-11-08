use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use usbip_audio::uac2::{Uac2Config, Uac2Device, Uac2Streams};
use usbip_audio::{MonotonicClock, Server};

#[derive(Default)]
struct Counter(Mutex<VecDeque<f32>>);

impl Uac2Streams for Counter {
    fn playback(&self, _frame: u64, samples: &[i32]) {
        if let Ok(mut queue) = self.0.lock() {
            for &sample in samples {
                if queue.len() >= 9600 {
                    queue.pop_front();
                }
                queue.push_back(sample as f32 / 2147483648.0);
            }
        }
    }

    fn capture(&self, _frame: u64, samples: &mut [i32]) {
        samples.fill(0);
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let host = cpal::default_host();
    let mut matching = host
        .output_devices()?
        .filter(|d| d.name().is_ok_and(|n| n.contains("Arctis Nova 7")));
    let output = matching.next().ok_or("Arctis Nova 7 output not found")?;
    if matching.next().is_some() {
        return Err("Multiple Arctis Nova 7 outputs found".into());
    }
    let supported = output
        .supported_output_configs()?
        .find(|c| {
            c.channels() == 2
                && c.sample_format() == cpal::SampleFormat::F32
                && c.min_sample_rate().0 <= 48000
                && c.max_sample_rate().0 >= 48000
        })
        .ok_or("Headset must support stereo 48 kHz f32 shared output")?;
    let config = supported.with_sample_rate(cpal::SampleRate(48000)).config();
    let game = Arc::new(Counter::default());
    let chat = Arc::new(Counter::default());
    let (game_out, chat_out) = (game.clone(), chat.clone());
    let game_gain = Arc::new(AtomicU8::new(100));
    let chat_gain = Arc::new(AtomicU8::new(100));
    let (game_mix, chat_mix) = (game_gain.clone(), chat_gain.clone());
    let stream = output.build_output_stream(
        &config,
        move |data: &mut [f32], _| {
            let mut a = game_out.0.lock().ok();
            let mut b = chat_out.0.lock().ok();
            let gain_a = f32::from(game_mix.load(Ordering::Relaxed)) / 100.0;
            let gain_b = f32::from(chat_mix.load(Ordering::Relaxed)) / 100.0;
            for sample in data {
                let x = a.as_mut().and_then(|q| q.pop_front()).unwrap_or(0.0);
                let y = b.as_mut().and_then(|q| q.pop_front()).unwrap_or(0.0);
                *sample = ((x * gain_a + y * gain_b) * 0.5).clamp(-1.0, 1.0);
            }
        },
        |error| eprintln!("Audio output error: {error}"),
        None,
    )?;
    stream.play()?;
    let api = hidapi::HidApi::new()?;
    let info = api
        .device_list()
        .find(|d| d.vendor_id() == 0x1038 && d.product_id() == 0x22a1 && d.usage_page() == 0xff00)
        .ok_or("ChatMix HID interface not found")?;
    let hid = info.open_device(&api)?;
    std::thread::spawn(move || {
        let mut buf = [0u8; 256];
        loop {
            match hid.read_timeout(&mut buf, 500) {
                Ok(n) => {
                    if let [0x45, game @ 0..=100, chat @ 0..=100, ..] = &buf[..n] {
                        game_gain.store(*game, Ordering::Relaxed);
                        chat_gain.store(*chat, Ordering::Relaxed);
                        println!("ChatMix game={game}% chat={chat}%");
                    }
                }
                Err(error) => {
                    eprintln!("ChatMix disconnected: {error}");
                    break;
                }
            }
        }
    });
    let server = Server::start(0, Arc::new(MonotonicClock::new()))?;
    let counters = [game, chat];
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
        let counter = counters[index].clone();
        let device = Uac2Device::new(config, counter.clone()).map_err(std::io::Error::other)?;
        server.add(&format!("1-{}", index + 1), Arc::new(device));
    }
    println!(
        "USB/IP server on localhost:{}; devices 1-1 (Game), 1-2 (Chat).",
        server.port()
    );
    println!("Mixing both streams to {} at half gain.", output.name()?);
    println!("Use Ctrl+C to stop.");
    loop {
        std::thread::sleep(Duration::from_secs(5));
        println!(
            "Received frames: game={}, chat={}",
            counters[0].0.lock().map(|q| q.len()).unwrap_or(0),
            counters[1].0.lock().map(|q| q.len()).unwrap_or(0)
        );
    }
}
