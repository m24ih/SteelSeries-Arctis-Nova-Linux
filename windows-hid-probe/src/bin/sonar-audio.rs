use std::error::Error;
use std::time::{Duration, Instant};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};

fn main() -> Result<(), Box<dyn Error>> {
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
        let mut game = None;
        let mut chat = None;
        for index in 0..devices.GetCount()? {
            let device = devices.Item(index)?;
            let properties = device.OpenPropertyStore(STGM_READ)?;
            let value = properties.GetValue(&PKEY_Device_FriendlyName)?;
            let ptr = PropVariantToStringAlloc(&value)?;
            let name = ptr.to_string()?;
            CoTaskMemFree(Some(ptr.0.cast()));
            if name.starts_with("SteelSeries Sonar - Gaming (")
                || name.starts_with("SteelSeries Sonar - Chat (")
            {
                let volume: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
                println!(
                    "{name}: {:.0}%",
                    volume.GetMasterVolumeLevelScalar()? * 100.0
                );
                let slot = if name.starts_with("SteelSeries Sonar - Gaming (") {
                    &mut game
                } else {
                    &mut chat
                };
                if slot.replace(volume).is_some() {
                    return Err(
                        "Multiple matching Sonar endpoints; refusing ambiguous selection".into(),
                    );
                }
            }
        }
        if std::env::args().any(|arg| arg == "--apply") {
            let game = game.ok_or("Sonar Gaming endpoint not found")?;
            let chat = chat.ok_or("Sonar Chat endpoint not found")?;
            let api = hidapi::HidApi::new()?;
            let info = api
                .device_list()
                .find(|d| {
                    d.vendor_id() == 0x1038 && d.product_id() == 0x22a1 && d.usage_page() == 0xff00
                })
                .ok_or("ChatMix HID interface not found")?;
            let device = info.open_device(&api)?;
            println!("Applying wheel events for 30 seconds; endpoint levels persist afterwards.");
            let deadline = Instant::now() + Duration::from_secs(30);
            let mut buffer = [0u8; 256];
            while Instant::now() < deadline {
                let len = device.read_timeout(&mut buffer, 250)?;
                if let [0x45, game_level @ 0..=100, chat_level @ 0..=100, ..] = &buffer[..len] {
                    game.SetMasterVolumeLevelScalar(
                        f32::from(*game_level) / 100.0,
                        std::ptr::null(),
                    )?;
                    chat.SetMasterVolumeLevelScalar(
                        f32::from(*chat_level) / 100.0,
                        std::ptr::null(),
                    )?;
                    println!("Applied: game={game_level}% chat={chat_level}%");
                }
            }
        }
    }
    Ok(())
}
