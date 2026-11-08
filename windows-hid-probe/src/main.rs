use hidapi::HidApi;
use std::error::Error;
use std::thread;
use std::time::{Duration, Instant};

const VENDOR: u16 = 0x1038;
const PRODUCT: u16 = 0x22a1;

fn main() -> Result<(), Box<dyn Error>> {
    let api = HidApi::new()?;
    let devices: Vec<_> = api
        .device_list()
        .filter(|d| d.vendor_id() == VENDOR && d.product_id() == PRODUCT)
        .collect();
    if devices.is_empty() {
        return Err("No matching Arctis Nova 7 device found".into());
    }
    for device in &devices {
        println!(
            "interface={} usage_page={:04x} usage={:04x}",
            device.interface_number(),
            device.usage_page(),
            device.usage()
        );
    }
    let mut listeners = Vec::new();
    for info in devices
        .into_iter()
        .filter(|d| d.usage_page() == 0xffc0 || d.usage_page() == 0xff00)
    {
        let device = match info.open_device(&api) {
            Ok(device) => device,
            Err(error) => {
                eprintln!(
                    "Could not open interface {}: {error}",
                    info.interface_number()
                );
                continue;
            }
        };
        let interface = info.interface_number();
        if info.usage_page() == 0xffc0 {
            let mut request = [0u8; 64];
            request[1] = 0xb0;
            match device.write(&request) {
                Ok(length) => {
                    println!("Interface {interface}: status request sent ({length} bytes)")
                }
                Err(error) => eprintln!("Interface {interface}: status request failed: {error}"),
            }
        }
        listeners.push(thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(45);
            let mut buffer = [0u8; 256];
            while Instant::now() < deadline {
                match device.read_timeout(&mut buffer, 250) {
                    Ok(0) => (),
                    Ok(length) => match &buffer[..length] {
                        [0x45, game, chat, ..] => println!("ChatMix: game={game}% chat={chat}%"),
                        [0xb0, wireless, battery, charging, ..] => println!(
                            "Status: wireless={wireless} battery={battery}% charging={charging}"
                        ),
                        report => println!("interface {interface}, {length} bytes: {report:02x?}"),
                    },
                    Err(error) => {
                        eprintln!("Interface {interface}: read failed: {error}");
                        break;
                    }
                }
            }
        }));
    }
    if listeners.is_empty() {
        return Err("No vendor HID interface could be opened".into());
    }
    println!("Listening on vendor interfaces for 45 seconds. Turn the ChatMix wheel now.");
    for listener in listeners {
        listener.join().map_err(|_| "Listener thread panicked")?;
    }
    Ok(())
}
