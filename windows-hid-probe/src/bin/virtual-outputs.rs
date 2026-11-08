use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use usbip_audio::uac2::{Uac2Config, Uac2Device, Uac2Streams};
use usbip_audio::{MonotonicClock, Server};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};

fn set_endpoint_volumes(game: u8, chat: u8) -> windows::core::Result<bool> {
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let mut found = [None, None];
        for index in 0..devices.GetCount()? {
            let device = devices.Item(index)?;
            let props = device.OpenPropertyStore(STGM_READ)?;
            let value = props.GetValue(&PKEY_Device_FriendlyName)?;
            let ptr = PropVariantToStringAlloc(&value)?;
            let name = ptr.to_string();
            CoTaskMemFree(Some(ptr.0.cast()));
            let name = name?;
            let slot = if name == "Speakers (Arctis Game prototype)" {
                Some(0)
            } else if name == "Speakers (Arctis Chat prototype)" {
                Some(1)
            } else {
                None
            };
            if let Some(slot) = slot {
                if found[slot].is_some() {
                    return Ok(false);
                }
                found[slot] = Some(device.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)?);
            }
        }
        if let [Some(game_endpoint), Some(chat_endpoint)] = found {
            game_endpoint.SetMasterVolumeLevelScalar(f32::from(game) / 100.0, std::ptr::null())?;
            chat_endpoint.SetMasterVolumeLevelScalar(f32::from(chat) / 100.0, std::ptr::null())?;
            return Ok(true);
        }
        Ok(false)
    }
}

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
    let stream = output.build_output_stream(
        &config,
        move |data: &mut [f32], _| {
            let mut a = game_out.0.lock().ok();
            let mut b = chat_out.0.lock().ok();
            for sample in data {
                let x = a.as_mut().and_then(|q| q.pop_front()).unwrap_or(0.0);
                let y = b.as_mut().and_then(|q| q.pop_front()).unwrap_or(0.0);
                *sample = ((x + y) * 0.5).clamp(-1.0, 1.0);
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
    let (game_volume, chat_volume) = (game_gain.clone(), chat_gain.clone());
    std::thread::spawn(move || {
        if let Err(error) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() } {
            eprintln!("Audio control initialization failed: {error}");
            return;
        }
        let mut last = None;
        loop {
            let levels = (
                game_volume.load(Ordering::Relaxed),
                chat_volume.load(Ordering::Relaxed),
            );
            if last != Some(levels) {
                match set_endpoint_volumes(levels.0, levels.1) {
                    Ok(true) => {
                        last = Some(levels);
                        println!(
                            "Windows endpoint levels: game={} chat={}",
                            levels.0, levels.1
                        );
                    }
                    Ok(false) => (),
                    Err(error) => eprintln!("Audio endpoint control failed: {error}"),
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    });
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
